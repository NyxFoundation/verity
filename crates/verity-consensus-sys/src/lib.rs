//! Zone B: Trusted shell (FFI bindings)
//!
//! Raw FFI bindings to Verity Consensus (Zone A).
//! Confines all `unsafe`.

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
