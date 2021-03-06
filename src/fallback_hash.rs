use crate::convert::{Convert};
use std::hash::{Hasher};
use arrayref::*;

///This constant come from Kunth's prng (Empirically it works better than those from splitmix32).
const MULTIPLE: u64 = 6364136223846793005;
const INCREMENT: u64 = 1442695040888963407;
const ROT: u32 = 23;

/// A `Hasher` for hashing an arbitrary stream of bytes.
///
/// Instances of [AHasher] represent state that is updated while hashing data.
///
/// Each method updates the internal state based on the new data provided. Once
/// all of the data has been provided, the resulting hash can be obtained by calling
/// `finish()`
///
/// [Clone] is also provided in case you wish to calculate hashes for two different items that
/// start with the same data.
///
#[derive(Debug, Clone)]
pub struct AHasher {
    buffer: u64
}

impl AHasher {
    /// Creates a new hasher keyed to the provided keys.
    #[inline]
    pub(crate) fn new_with_keys(key0: u64, key1: u64) -> AHasher {
        AHasher { buffer: key0 ^ key1.rotate_left(ROT) }
    }

    /// This update function has the goal of updating the buffer with a single multiply
    /// FxHash does this but is venerable to attack. To avoid this input needs to be masked to with an unpredictable value.
    /// However other hashes such as murmurhash have taken that approach but were found venerable to attack.
    /// The attack was based on the idea of reversing the pre-mixing (Which is necessarily reversible otherwise
    /// bits would be lost) then placing a difference in the highest bit before the multiply. Because a multiply
    /// can never affect the bits to the right of it. This version avoids this vulnerability by rotating and
    /// performing a second multiply. This makes it impossible for an attacker to place a single bit
    /// difference between two blocks so as to cancel each other. (While the transform is still reversible if you know the key)
    ///
    /// The key needs to be incremented between consecutive calls to prevent (a,b) from hashing the same as (b,a).
    /// The adding of the increment is moved to the bottom rather than the top. This allows one less add to be
    /// performed overall, but more importantly, it follows the multiply, which is expensive. So the CPU can
    /// run another operation afterwords if does not depend on the output of the multiply operation.
    ///
    /// The update of the buffer to perform the second multiply is moved from the end to the beginning of the method.
    /// This has the effect of causing the next call to update to perform he second multiply. For the final
    /// update this is performed in the finalize method. This might seem wasteful, but its actually an optimization.
    /// If the method get's inlined into the caller where it is being invoked on a single primitive, the first call
    /// to update the buffer will be operating on constants and the compiler will optimize it out, by replacing it with
    /// the result.
    #[inline(always)]
    fn update(&mut self, new_data: u64) {
        let existing = self.buffer.wrapping_mul(MULTIPLE).rotate_left(ROT).wrapping_mul(MULTIPLE);
        self.buffer = existing ^ new_data;
    }

    /// This is similar to the above update function (see it's description). But is designed to run in a loop
    /// that will be unrolled and vectorized. So instead of using the buffer, it uses a 'key' that it updates
    /// and returns. The buffer is only xored at the end. This structure is so that when the method is inlined,
    /// the compiler will unroll any loop this gets placed in and the loop can be automatically vectorized
    /// and the rotates, xors, and multiplies can be paralleled.
    #[inline(always)]
    fn ordered_update(&mut self, new_data: u64, key: u64) -> u64 {
        self.buffer ^= (new_data ^ key).wrapping_mul(MULTIPLE).rotate_left(ROT).wrapping_mul(MULTIPLE);
        key.wrapping_add(INCREMENT)
    }
}

/// Provides methods to hash all of the primitive types.
impl Hasher for AHasher {

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.update(i as u64);
    }

    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.update(i as u64);
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.update(i as u64);
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.update(i as u64);
    }

    #[inline]
    fn write_u128(&mut self, i: u128) {
        let data: [u64;2] = i.convert();
        self.update(data[0]);
        self.update(data[1]);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.write_u64(i as u64);
    }

    #[inline]
    fn write(&mut self, input: &[u8]) {
        let mut data = input;
        let length = data.len() as u64;
        //Needs to be an add rather than an xor because otherwise it could be canceled with carefully formed input.
        self.buffer = self.buffer.wrapping_add(length);
        //A 'binary search' on sizes reduces the number of comparisons.
        if data.len() > 8 {
            let mut key: u64 = self.buffer;
            while data.len() > 16 {
                let (block, rest) = data.split_at(8);
                let val: u64 = as_array!(block, 8).convert();
                key = self.ordered_update(val, key);
                data = rest;
            }
            let val: u64 = (*array_ref!(data, 0, 8)).convert();
            self.ordered_update(val, key);
            let val: u64 = (*array_ref!(data, data.len()-8, 8)).convert();
            self.update(val);
        } else {
            if data.len() >= 2 {
                if data.len() >= 4 {
                    let block: [u32; 2] = [(*array_ref!(data, 0, 4)).convert(),
                        (*array_ref!(data, data.len()-4, 4)).convert()];
                    self.update(block.convert());
                } else {
                    let block: [u16; 2] = [(*array_ref!(data, 0, 2)).convert(),
                        (*array_ref!(data, data.len()-2, 2)).convert()];
                    let val: u32 = block.convert();
                    self.update(val as u64);
                }
            } else {
                if data.len() >= 1 {
                    self.update(data[0] as u64);
                }
            }
        }
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.buffer.wrapping_mul(MULTIPLE).rotate_left(ROT).wrapping_mul(MULTIPLE)
    }
}


#[cfg(test)]
mod tests {
    use crate::convert::Convert;
    use crate::fallback_hash::*;

    #[test]
    fn test_hash() {
        let mut hasher = AHasher::new_with_keys(0,0);
        let value: u64 = 1 << 32;
        hasher.update(value);
        let result = hasher.buffer;
        let mut hasher = AHasher::new_with_keys(0,0);
        let value2: u64 = 1;
        hasher.update(value2);
        let result2 = hasher.buffer;
        let result: [u8; 8] = result.convert();
        let result2: [u8; 8] = result2.convert();
        assert_ne!(hex::encode(result), hex::encode(result2));
    }

    #[test]
    fn test_conversion() {
        let input: &[u8] = "dddddddd".as_bytes();
        let bytes: u64 = as_array!(input, 8).convert();
        assert_eq!(bytes, 0x6464646464646464);
    }
}
