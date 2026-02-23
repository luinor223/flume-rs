//! 1:0/1 record filtering operator.

use crate::{Collector, FlumeResult, Operator, Record};

/// Passes records where `predicate(&T)` is true.
pub struct FilterOperator<F> {
    predicate: F,
}

impl<F> FilterOperator<F> {
    pub fn new(predicate: F) -> Self {
        Self { predicate }
    }
}

impl<T, F> Operator<T, T> for FilterOperator<F>
where
    T: Send,
    F: FnMut(&T) -> bool + Send,
{
    fn process_record(
        &mut self,
        record: Record<T>,
        collector: &mut dyn Collector<T>,
    ) -> FlumeResult<()> {
        if (self.predicate)(&record.value) {
            collector.collect(record)?;
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
    fn test_filter_passes() {
        let mut op = FilterOperator::new(|x: &i32| *x > 3);
        let mut collector = VecCollector::new();

        op.process_record(Record::new(5, EventTimestamp::new(100)), &mut collector)
            .unwrap();
        op.process_record(Record::new(2, EventTimestamp::new(200)), &mut collector)
            .unwrap();
        op.process_record(Record::new(10, EventTimestamp::new(300)), &mut collector)
            .unwrap();

        assert_eq!(collector.records.len(), 2);
        assert_eq!(collector.records[0].value, 5);
        assert_eq!(collector.records[1].value, 10);
    }
}
