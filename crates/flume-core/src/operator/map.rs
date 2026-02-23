//! 1:1 record transformation operator.

use crate::{Collector, FlumeResult, Operator, Record};

/// Transforms each record via `f(In) -> Out`.
pub struct MapOperator<F> {
    f: F,
}

impl<F> MapOperator<F> {
    pub fn new(f: F) -> Self {
        Self { f }
    }
}

impl<In, Out, F> Operator<In, Out> for MapOperator<F>
where
    In: Send,
    Out: Send,
    F: FnMut(In) -> Out + Send,
{
    fn process_record(
        &mut self,
        record: Record<In>,
        collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()> {
        let mapped = record.map(&mut self.f);
        collector.collect(mapped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CheckpointBarrier, EventTimestamp, Watermark};

    struct VecCollector<T> {
        records: Vec<Record<T>>,
    }

    impl<T> VecCollector<T> {
        fn new() -> Self {
            Self {
                records: Vec::new(),
            }
        }
    }

    impl<T: Send> Collector<T> for VecCollector<T> {
        fn collect(&mut self, record: Record<T>) -> FlumeResult<()> {
            self.records.push(record);
            Ok(())
        }
        fn collect_watermark(&mut self, _: Watermark) -> FlumeResult<()> {
            Ok(())
        }
        fn collect_barrier(&mut self, _: CheckpointBarrier) -> FlumeResult<()> {
            Ok(())
        }
    }

    #[test]
    fn test_map_operator() {
        let mut op = MapOperator::new(|x: i32| x * 2);
        let mut collector = VecCollector::new();

        let record = Record::new(5, EventTimestamp::new(100));
        op.process_record(record, &mut collector).unwrap();

        assert_eq!(collector.records.len(), 1);
        assert_eq!(collector.records[0].value, 10);
        assert_eq!(collector.records[0].timestamp, EventTimestamp::new(100));
    }
}
