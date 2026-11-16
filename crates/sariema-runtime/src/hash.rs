// SPDX-License-Identifier: Apache-2.0

//! The state hash: FNV-1a 64 over a canonical byte stream, so equal states
//! hash equal on every run and platform. Floats are hashed by bit pattern.

pub struct StateHasher(u64);

impl Default for StateHasher {
    fn default() -> StateHasher {
        StateHasher(0xcbf2_9ce4_8422_2325)
    }
}

impl StateHasher {
    pub fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }

    pub fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }

    pub fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }

    pub fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }

    pub fn vec3(&mut self, v: glam::Vec3) {
        v.to_array().into_iter().for_each(|c| self.f32(c));
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_fnv1a_64() {
        let mut h = StateHasher::default();
        h.bytes(b"a");
        assert_eq!(h.finish(), 0xaf63_dc4c_8601_ec8c);
    }
}
