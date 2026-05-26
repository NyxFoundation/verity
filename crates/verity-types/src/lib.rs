//! Zone B: Trusted shell
//!
//! Manufactures clean, typed, already-verified inputs for Verity Consensus,
//! owns the consensus state and fork-choice view as a single writer, and
//! threads immutable values through Verity Consensus.

pub fn add(left: usize, right: usize) -> usize {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
