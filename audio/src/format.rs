//! Sample formats, and samples read as floats: every format a program's sound comes in (PulseAudio's, ALSA's and
//! WAVE's between them), each sample to `[-1, 1]`.

/// How one sample is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sample {
    U8,
    S16Le,
    S16Be,
    /// 24 bits in 3 bytes.
    S24Le,
    S24Be,
    /// 24 bits in the low 3 bytes of 4.
    S24In32Le,
    S24In32Be,
    S32Le,
    S32Be,
    F32Le,
    F32Be,
    /// G.711's companded 8-bit samples.
    Alaw,
    Ulaw,
}

impl Sample {
    pub const fn bytes(self) -> usize {
        match self {
            Sample::U8 | Sample::Alaw | Sample::Ulaw => 1,
            Sample::S16Le | Sample::S16Be => 2,
            Sample::S24Le | Sample::S24Be => 3,
            _ => 4,
        }
    }

    /// One sample, from its bytes (`self.bytes()` of them), in `[-1, 1]` (a float sample as it is).
    pub fn read(self, b: &[u8]) -> f32 {
        let s24 = |x: i32| (x << 8 >> 8) as f32 / 8_388_608.0;
        match self {
            Sample::U8 => (f32::from(b[0]) - 128.0) / 128.0,
            Sample::S16Le => f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0,
            Sample::S16Be => f32::from(i16::from_be_bytes([b[0], b[1]])) / 32768.0,
            Sample::S24Le => s24(i32::from_le_bytes([b[0], b[1], b[2], 0])),
            Sample::S24Be => s24(i32::from_be_bytes([0, b[0], b[1], b[2]])),
            Sample::S24In32Le => s24(i32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            Sample::S24In32Be => s24(i32::from_be_bytes([b[0], b[1], b[2], b[3]])),
            Sample::S32Le => (f64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])) / 2_147_483_648.0) as f32,
            Sample::S32Be => (f64::from(i32::from_be_bytes([b[0], b[1], b[2], b[3]])) / 2_147_483_648.0) as f32,
            Sample::F32Le => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            Sample::F32Be => f32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            Sample::Alaw => f32::from(alaw(b[0])) / 32768.0,
            Sample::Ulaw => f32::from(ulaw(b[0])) / 32768.0,
        }
    }
}

/// A stream's samples: how each is stored, how many channels a frame has, and how many frames a second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Format {
    pub sample: Sample,
    pub channels: u8,
    pub rate: u32,
}

/// The most channels a stream may have, and the highest rate.
pub const MAX_CHANNELS: u8 = 32;
pub const MAX_RATE: u32 = 384_000;

impl Format {
    pub const fn new(sample: Sample, channels: u8, rate: u32) -> Format {
        Format { sample, channels, rate }
    }

    /// The bytes of one frame: a sample for each channel.
    pub const fn frame_bytes(&self) -> usize {
        self.sample.bytes() * self.channels as usize
    }

    pub fn is_valid(&self) -> bool {
        (1..=MAX_CHANNELS).contains(&self.channels) && (1..=MAX_RATE).contains(&self.rate)
    }

    /// One frame, from its bytes, as stereo: mono to both sides; more than two channels folded down, taking them in
    /// the order Linux, WAVE and Windows all number them (front left and right, centre, LFE, then the rear and side
    /// pairs), the centre and the rest at -3 dB, the LFE left out.
    pub fn stereo(&self, frame: &[u8]) -> [f32; 2] {
        let n = self.sample.bytes();
        let at = |c: usize| self.sample.read(&frame[c * n..c * n + n]);
        match self.channels {
            1 => {
                let v = at(0);
                [v, v]
            }
            2 => [at(0), at(1)],
            c => {
                const HALF: f32 = std::f32::consts::FRAC_1_SQRT_2;
                let (mut l, mut r) = (at(0), at(1));
                if c >= 3 {
                    let centre = at(2) * HALF;
                    l += centre;
                    r += centre;
                }
                // Pairs after the LFE: left to the left, right to the right.
                let mut k = 4;
                while k + 1 < c as usize {
                    l += at(k) * HALF;
                    r += at(k + 1) * HALF;
                    k += 2;
                }
                [l, r]
            }
        }
    }
}

/// G.711 A-law, as a 16-bit sample.
fn alaw(a: u8) -> i16 {
    let a = a ^ 0x55;
    let exponent = (a >> 4) & 7;
    let mantissa = i16::from(a & 0x0f);
    let magnitude = if exponent == 0 { (mantissa << 4) + 8 } else { ((mantissa << 4) + 0x108) << (exponent - 1) };
    if a & 0x80 != 0 { magnitude } else { -magnitude }
}

/// G.711 µ-law, as a 16-bit sample.
fn ulaw(u: u8) -> i16 {
    let u = !u;
    let exponent = (u >> 4) & 7;
    let mantissa = i16::from(u & 0x0f);
    let magnitude = (((mantissa << 3) + 0x84) << exponent) - 0x84;
    if u & 0x80 != 0 { -magnitude } else { magnitude }
}
