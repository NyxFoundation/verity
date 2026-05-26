//! Zone B: Trusted shell
//!
//! The single writer that owns the consensus state and the fork-choice store.
//! Coordinates the `State` and `Store` aggregates under one consistency boundary.

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
