//! 1:N record expansion operator.

use crate::{Collector, FlumeResult, Operator, Record};

/// Expands each record via `f(In) -> Vec<Out>`.
pub struct FlatMapOperator<F> {
    f: F,
}

impl<F> FlatMapOperator<F> {
    pub fn new(f: F) -> Self {
        Self { f }
    }
}

impl<In, Out, F> Operator<In, Out> for FlatMapOperator<F>
where
    In: Send,
    Out: Send,
    F: FnMut(In) -> Vec<Out> + Send,
{
    fn process_record(
        &mut self,
        record: Record<In>,
        collector: &mut dyn Collector<Out>,
    ) -> FlumeResult<()> {
        let outputs = (self.f)(record.value);
        for value in outputs {
            collector.collect(Record::new(value, record.timestamp))?;
        }
        Ok(())
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
    fn test_flat_map() {
        let mut op = FlatMapOperator::new(|x: i32| vec![x, x * 10, x * 100]);
        let mut collector = VecCollector::new();

        op.process_record(Record::new(3, EventTimestamp::new(100)), &mut collector)
            .unwrap();

        assert_eq!(collector.records.len(), 3);
        assert_eq!(collector.records[0].value, 3);
        assert_eq!(collector.records[1].value, 30);
        assert_eq!(collector.records[2].value, 300);
    }

    #[test]
    fn test_flat_map_empty() {
        let mut op = FlatMapOperator::new(|_: i32| Vec::<i32>::new());
        let mut collector = VecCollector::new();

        op.process_record(Record::new(1, EventTimestamp::new(100)), &mut collector)
            .unwrap();

        assert!(collector.records.is_empty());
    }
}
