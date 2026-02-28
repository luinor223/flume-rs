//! A two-variant sum type for merging two input streams.
//!
//! Used by connected streams to tag elements from two sources so a
//! single `Operator<Either<In1, In2>, Out>` can dispatch between them
//! without requiring a new trait or `TaskExecutor` changes.

/// A value from one of two input streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Either<L, R> {
    Left(L),
    Right(R),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_either_left() {
        let e: Either<i32, String> = Either::Left(42);
        assert_eq!(e, Either::Left(42));
    }

    #[test]
    fn test_either_right() {
        let e: Either<i32, String> = Either::Right("hello".to_string());
        assert_eq!(e, Either::Right("hello".to_string()));
    }

    #[test]
    fn test_either_clone() {
        let e: Either<i32, i32> = Either::Left(10);
        let cloned = e.clone();
        assert_eq!(e, cloned);
    }

    #[test]
    fn test_either_debug() {
        let e: Either<i32, &str> = Either::Left(1);
        let debug = format!("{e:?}");
        assert!(debug.contains("Left"));
    }
}
