//! Named function traits as alternatives to closures for reusable/configurable logic.

/// Named alternative to `FnMut(In) -> Out` for map operations.
pub trait MapFunction<In, Out>: Send {
    fn map(&mut self, value: In) -> Out;
}

/// Named alternative to `FnMut(&T) -> bool` for filter operations.
pub trait FilterFunction<T>: Send {
    fn filter(&self, value: &T) -> bool;
}

/// Named alternative to `FnMut(In) -> Vec<Out>` for flat_map operations.
pub trait FlatMapFunction<In, Out>: Send {
    fn flat_map(&mut self, value: In, out: &mut Vec<Out>);
}

/// Named alternative for reduce operations on keyed streams.
pub trait ReduceFunction<T>: Send {
    fn reduce(&self, a: T, b: T) -> T;
}
