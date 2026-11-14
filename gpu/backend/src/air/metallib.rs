//! `.metallib`: the libraries Metal loads with `newLibraryWithData:`.
//!
//! A library is an 88-byte header, a function list, and three sections the list points into: public metadata,
//! private metadata, and bitcode, one module per function. Each function's entry is a run of tagged fields, `NAME`,
//! `TYPE`, `HASH` (the SHA-256 of its bitcode), `MDSZ` (the bitcode's size), `OFFT` (its offsets into the three
//! sections) and `VERS` (its AIR and MSL versions), ending in `ENDT`, as xcrun's `metallib` writes them.

use super::module::Stage;
use super::bitcode::air_version;
use super::sha256::sha256;

const HEADER: u64 = 88;
/// `TYPE`'s value for each kind of function: vertex 0, fragment 1, kernel 2, as xcrun writes them.
fn type_tag(stage: Stage) -> u8 {
    match stage {
        Stage::Vertex => 0,
        Stage::Fragment => 1,
        Stage::Kernel => 2,
    }
}

fn tags(out: &mut Vec<u8>, tags: &[(&[u8; 4], &[u8])]) {
    for (name, data) in tags {
        out.extend_from_slice(*name);
        out.extend_from_slice(&(data.len() as u16).to_le_bytes());
        out.extend_from_slice(data);
    }
    out.extend_from_slice(b"ENDT");
}

/// The library format xcrun writes for a deployment target (measured): 2.7 for macOS 13 and 14, 2.8 for 15, 2.9
/// for 26.
fn file_version(os: (u32, u32)) -> (u16, u16) {
    match os.0 {
        26.. => (2, 9),
        15.. => (2, 8),
        _ => (2, 7),
    }
}

/// A library of one function, `name`, whose module is `air` (a wrapped `.air` file).
pub(crate) fn build(name: &str, air: &[u8], os: (u32, u32), language: (u32, u32), stage: Stage) -> Vec<u8> {
    let le16 = |v: u32| (v as u16).to_le_bytes();
    let (a_major, a_minor) = air_version(os);
    let mut vers = Vec::new();
    for v in [a_major, a_minor, language.0, language.1] {
        vers.extend_from_slice(&le16(v));
    }
    let mut fname = name.as_bytes().to_vec();
    fname.push(0);
    let hash = sha256(air);
    let mdsz = (air.len() as u64).to_le_bytes();
    let mut offt = Vec::new();
    for off in [0u64, 0, 0] {
        offt.extend_from_slice(&off.to_le_bytes()); // public metadata, private metadata, bitcode: each its first
    }

    let mut entry = Vec::new();
    tags(&mut entry, &[(b"NAME", &fname), (b"TYPE", &[type_tag(stage)]), (b"HASH", &hash), (b"MDSZ", &mdsz), (b"OFFT", &offt), (b"VERS", &vers)]);
    let mut entries = ((4 + entry.len()) as u32).to_le_bytes().to_vec();
    entries.extend_from_slice(&entry);

    let mut ext = Vec::new();
    tags(&mut ext, &[]);
    // Each metadata section holds the function's tags (none) after their size.
    let mut public = Vec::new();
    let mut none = Vec::new();
    tags(&mut none, &[]);
    public.extend_from_slice(&(none.len() as u32).to_le_bytes());
    public.extend_from_slice(&none);
    let private = public.clone();

    let list = HEADER;
    let public_at = list + 4 + entries.len() as u64 + ext.len() as u64;
    let private_at = public_at + public.len() as u64;
    let code_at = private_at + private.len() as u64;
    let total = code_at + air.len() as u64;

    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"MTLB");
    out.extend_from_slice(&0x8001u16.to_le_bytes()); // the platform: macOS
    let (f_major, f_minor) = file_version(os);
    out.extend_from_slice(&f_major.to_le_bytes());
    out.extend_from_slice(&f_minor.to_le_bytes());
    out.push(0); // an executable library
    out.push(0x81); // the target OS: macOS
    out.extend_from_slice(&le16(os.0));
    out.extend_from_slice(&le16(os.1));
    for v in [total, list, entries.len() as u64, public_at, public.len() as u64, private_at, private.len() as u64, code_at, air.len() as u64] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    debug_assert_eq!(out.len() as u64, HEADER);
    out.extend_from_slice(&1u32.to_le_bytes()); // one function
    out.extend_from_slice(&entries);
    out.extend_from_slice(&ext);
    out.extend_from_slice(&public);
    out.extend_from_slice(&private);
    out.extend_from_slice(air);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_points_at_its_sections() {
        let air = [7u8; 32];
        let lib = build("k", &air, (13, 0), (3, 0), Stage::Kernel);
        let at = |i: usize| u64::from_le_bytes(lib[i..i + 8].try_into().unwrap()) as usize;
        assert_eq!(&lib[..4], b"MTLB");
        assert_eq!(at(16), lib.len());
        assert_eq!(&lib[at(72)..at(72) + at(80)], &air);
        assert_eq!(u32::from_le_bytes(lib[88..92].try_into().unwrap()), 1);
    }
}
