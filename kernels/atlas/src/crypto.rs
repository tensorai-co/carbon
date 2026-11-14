//! ChaCha20 and Poly1305, and the two as one authenticated cipher (RFC 8439): what DPAPI (`CryptProtectData`) and
//! `CryptProtectMemory` keep a program's secrets with, under keys only the machine has.

/// ChaCha20's block for a key, counter and nonce: 64 bytes of key stream.
fn block(key: &[u8; 32], counter: u32, nonce: &[u8; 12]) -> [u8; 64] {
    let word = |b: &[u8], i: usize| u32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]);
    let mut s = [0u32; 16];
    s[..4].copy_from_slice(&[0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574]);
    for i in 0..8 {
        s[4 + i] = word(key, i);
    }
    s[12] = counter;
    for i in 0..3 {
        s[13 + i] = word(nonce, i);
    }
    let mut x = s;
    fn quarter(x: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
        x[a] = x[a].wrapping_add(x[b]);
        x[d] = (x[d] ^ x[a]).rotate_left(16);
        x[c] = x[c].wrapping_add(x[d]);
        x[b] = (x[b] ^ x[c]).rotate_left(12);
        x[a] = x[a].wrapping_add(x[b]);
        x[d] = (x[d] ^ x[a]).rotate_left(8);
        x[c] = x[c].wrapping_add(x[d]);
        x[b] = (x[b] ^ x[c]).rotate_left(7);
    }
    for _ in 0..10 {
        quarter(&mut x, 0, 4, 8, 12);
        quarter(&mut x, 1, 5, 9, 13);
        quarter(&mut x, 2, 6, 10, 14);
        quarter(&mut x, 3, 7, 11, 15);
        quarter(&mut x, 0, 5, 10, 15);
        quarter(&mut x, 1, 6, 11, 12);
        quarter(&mut x, 2, 7, 8, 13);
        quarter(&mut x, 3, 4, 9, 14);
    }
    let mut out = [0u8; 64];
    for i in 0..16 {
        out[4 * i..4 * i + 4].copy_from_slice(&x[i].wrapping_add(s[i]).to_le_bytes());
    }
    out
}

/// XORs `data` with ChaCha20's stream from block `counter`: encrypting and decrypting are the same.
pub fn chacha20(key: &[u8; 32], counter: u32, nonce: &[u8; 12], data: &mut [u8]) {
    for (i, chunk) in data.chunks_mut(64).enumerate() {
        let stream = block(key, counter.wrapping_add(i as u32), nonce);
        for (b, s) in chunk.iter_mut().zip(stream) {
            *b ^= s;
        }
    }
}

/// Poly1305's tag of `message` under a one-time key, in 26-bit limbs.
fn poly1305(key: &[u8; 32], message: &[u8]) -> [u8; 16] {
    let le = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let r0 = le(&key[0..]) & 0x03ff_ffff;
    let r1 = (le(&key[3..]) >> 2) & 0x03ff_ff03;
    let r2 = (le(&key[6..]) >> 4) & 0x03ff_c0ff;
    let r3 = (le(&key[9..]) >> 6) & 0x03f0_3fff;
    let r4 = (le(&key[12..]) >> 8) & 0x000f_ffff;
    let (s1, s2, s3, s4) = (r1 * 5, r2 * 5, r3 * 5, r4 * 5);
    let mut h = [0u32; 5];
    for chunk in message.chunks(16) {
        let mut m = [0u8; 17];
        m[..chunk.len()].copy_from_slice(chunk);
        m[chunk.len()] = 1;
        h[0] += le(&m[0..]) & 0x03ff_ffff;
        h[1] += (le(&m[3..]) >> 2) & 0x03ff_ffff;
        h[2] += (le(&m[6..]) >> 4) & 0x03ff_ffff;
        h[3] += (le(&m[9..]) >> 6) & 0x03ff_ffff;
        h[4] += (le(&m[12..]) >> 8) | u32::from(m[16]) << 24;
        let m64 = |a: u32, b: u32| u64::from(a) * u64::from(b);
        let d0 = m64(h[0], r0) + m64(h[1], s4) + m64(h[2], s3) + m64(h[3], s2) + m64(h[4], s1);
        let mut d1 = m64(h[0], r1) + m64(h[1], r0) + m64(h[2], s4) + m64(h[3], s3) + m64(h[4], s2);
        let mut d2 = m64(h[0], r2) + m64(h[1], r1) + m64(h[2], r0) + m64(h[3], s4) + m64(h[4], s3);
        let mut d3 = m64(h[0], r3) + m64(h[1], r2) + m64(h[2], r1) + m64(h[3], r0) + m64(h[4], s4);
        let mut d4 = m64(h[0], r4) + m64(h[1], r3) + m64(h[2], r2) + m64(h[3], r1) + m64(h[4], r0);
        let mut c = (d0 >> 26) as u32;
        h[0] = d0 as u32 & 0x03ff_ffff;
        d1 += u64::from(c);
        c = (d1 >> 26) as u32;
        h[1] = d1 as u32 & 0x03ff_ffff;
        d2 += u64::from(c);
        c = (d2 >> 26) as u32;
        h[2] = d2 as u32 & 0x03ff_ffff;
        d3 += u64::from(c);
        c = (d3 >> 26) as u32;
        h[3] = d3 as u32 & 0x03ff_ffff;
        d4 += u64::from(c);
        c = (d4 >> 26) as u32;
        h[4] = d4 as u32 & 0x03ff_ffff;
        h[0] += c * 5;
        c = h[0] >> 26;
        h[0] &= 0x03ff_ffff;
        h[1] += c;
    }
    // Fully carried, then reduced mod 2^130 - 5.
    let mut c = h[1] >> 26;
    h[1] &= 0x03ff_ffff;
    for limb in &mut h[2..5] {
        *limb += c;
        c = *limb >> 26;
        *limb &= 0x03ff_ffff;
    }
    h[0] += c * 5;
    c = h[0] >> 26;
    h[0] &= 0x03ff_ffff;
    h[1] += c;
    let mut g = [0u32; 5];
    g[0] = h[0].wrapping_add(5);
    c = g[0] >> 26;
    g[0] &= 0x03ff_ffff;
    for i in 1..5 {
        g[i] = h[i].wrapping_add(c);
        c = g[i] >> 26;
        g[i] &= 0x03ff_ffff;
    }
    g[4] = g[4].wrapping_sub(1 << 26);
    // h if h < p, else h - p: chosen without branching on the secret.
    let mask = (g[4] >> 31).wrapping_sub(1);
    for i in 0..5 {
        h[i] = (h[i] & !mask) | (g[i] & mask);
    }
    let h0 = h[0] | h[1] << 26;
    let h1 = (h[1] >> 6) | h[2] << 20;
    let h2 = (h[2] >> 12) | h[3] << 14;
    let h3 = (h[3] >> 18) | h[4] << 8;
    let mut f = u64::from(h0) + u64::from(le(&key[16..]));
    let mut tag = [0u8; 16];
    tag[0..4].copy_from_slice(&(f as u32).to_le_bytes());
    f = u64::from(h1) + u64::from(le(&key[20..])) + (f >> 32);
    tag[4..8].copy_from_slice(&(f as u32).to_le_bytes());
    f = u64::from(h2) + u64::from(le(&key[24..])) + (f >> 32);
    tag[8..12].copy_from_slice(&(f as u32).to_le_bytes());
    f = u64::from(h3) + u64::from(le(&key[28..])) + (f >> 32);
    tag[12..16].copy_from_slice(&(f as u32).to_le_bytes());
    tag
}

/// The tag over the extra data and the ciphertext, each padded to 16 bytes, then their lengths.
fn aead_tag(key: &[u8; 32], nonce: &[u8; 12], extra: &[u8], ciphertext: &[u8]) -> [u8; 16] {
    let one_time: [u8; 32] = block(key, 0, nonce)[..32].try_into().unwrap_or([0; 32]);
    let mut mac = Vec::with_capacity(extra.len() + ciphertext.len() + 48);
    for part in [extra, ciphertext] {
        mac.extend(part);
        mac.resize(mac.len().next_multiple_of(16), 0);
    }
    mac.extend((extra.len() as u64).to_le_bytes());
    mac.extend((ciphertext.len() as u64).to_le_bytes());
    poly1305(&one_time, &mac)
}

/// ChaCha20-Poly1305: the ciphertext, then its 16-byte tag.
pub fn seal(key: &[u8; 32], nonce: &[u8; 12], extra: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let mut out = plaintext.to_vec();
    chacha20(key, 1, nonce, &mut out);
    let tag = aead_tag(key, nonce, extra, &out);
    out.extend(tag);
    out
}

/// The plaintext, if the tag is right.
pub fn open(key: &[u8; 32], nonce: &[u8; 12], extra: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    let (ciphertext, tag) = sealed.split_at_checked(sealed.len().checked_sub(16)?)?;
    let expected = aead_tag(key, nonce, extra, ciphertext);
    // Compared in full, whatever differs first.
    if expected.iter().zip(tag).fold(0u8, |d, (a, b)| d | (a ^ b)) != 0 {
        return None;
    }
    let mut out = ciphertext.to_vec();
    chacha20(key, 1, nonce, &mut out);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.chars().filter(char::is_ascii_hexdigit).collect();
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn rfc_8439_block_and_mac() {
        // 2.3.2: the block function.
        let key: [u8; 32] = core::array::from_fn(|i| i as u8);
        let nonce: [u8; 12] = hex("000000090000004a00000000").try_into().unwrap();
        assert_eq!(block(&key, 1, &nonce)[..16], hex("10f1e7e4d13b5915500fdd1fa32071c4")[..]);
        // 2.5.2: Poly1305.
        let mkey: [u8; 32] = hex("85d6be7857556d337f4452fe42d506a80103808afb0db2fd4abff6af4149f51b").try_into().unwrap();
        assert_eq!(poly1305(&mkey, b"Cryptographic Forum Research Group").to_vec(), hex("a8061dc1305136c6c22b8baf0c0127a9"));
    }

    #[test]
    fn rfc_8439_aead() {
        // 2.8.2.
        let key: [u8; 32] = hex("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f").try_into().unwrap();
        let nonce: [u8; 12] = hex("070000004041424344454647").try_into().unwrap();
        let extra = hex("50515253c0c1c2c3c4c5c6c7");
        let text = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        let sealed = seal(&key, &nonce, &extra, text);
        assert_eq!(sealed[..16], hex("d31a8d34648e60db7b86afbc53ef7ec2")[..]);
        assert_eq!(sealed[sealed.len() - 16..], hex("1ae10b594f09e26a7e902ecbd0600691")[..]);
        assert_eq!(open(&key, &nonce, &extra, &sealed).as_deref(), Some(&text[..]));
        let mut bad = sealed.clone();
        bad[3] ^= 1;
        assert!(open(&key, &nonce, &extra, &bad).is_none());
        assert!(open(&key, &nonce, b"other", &sealed).is_none());
    }
}
