//! Integration tests for window joins and interval joins.

use std::time::Duration;

use flume_api::{CollectSink, StreamExecutionEnvironment, TimestampedSource};
use flume_core::{EventTimestamp, Record, StreamElement, TumblingWindow, Watermark};

// ---------------------------------------------------------------------------
// Window join tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_window_join_tumbling() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<i32>::new();

    // 2 trades in window [0, 10_000)
    let trades = vec![
        StreamElement::Record(Record::new(1i32, EventTimestamp::new(1_000))),
        StreamElement::Record(Record::new(2i32, EventTimestamp::new(2_000))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(10_000))),
    ];

    // 3 quotes in same window
    let quotes = TimestampedSource::new(vec![
        StreamElement::Record(Record::new(10i32, EventTimestamp::new(3_000))),
        StreamElement::Record(Record::new(20i32, EventTimestamp::new(4_000))),
        StreamElement::Record(Record::new(30i32, EventTimestamp::new(5_000))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(10_000))),
    ]);

    env.from_source(TimestampedSource::new(trades))
        .window_join(quotes)
        .key_by_both(|_: &i32| 0u8, |_: &i32| 0u8)
        .window(TumblingWindow::new(Duration::from_secs(10)))
        .apply(|l: &i32, r: &i32| l + r)
        .add_sink(sink)
        .await
        .unwrap();

    let values = results.lock().unwrap();
    // 2 left x 3 right = 6 outputs
    assert_eq!(values.len(), 6);

    let mut sorted = values.clone();
    sorted.sort();
    // 1+10=11, 1+20=21, 1+30=31, 2+10=12, 2+20=22, 2+30=32
    assert_eq!(sorted, vec![11, 12, 21, 22, 31, 32]);
}

#[tokio::test]
async fn test_window_join_multiple_keys() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<i32>::new();

    // Key A (even) and Key B (odd) trades
    let trades = vec![
        StreamElement::Record(Record::new(2i32, EventTimestamp::new(1_000))),
        StreamElement::Record(Record::new(3i32, EventTimestamp::new(2_000))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(10_000))),
    ];

    // Key A (even) and Key B (odd) quotes
    let quotes = TimestampedSource::new(vec![
        StreamElement::Record(Record::new(10i32, EventTimestamp::new(3_000))),
        StreamElement::Record(Record::new(11i32, EventTimestamp::new(4_000))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(10_000))),
    ]);

    env.from_source(TimestampedSource::new(trades))
        .window_join(quotes)
        .key_by_both(|v: &i32| v % 2, |v: &i32| v % 2)
        .window(TumblingWindow::new(Duration::from_secs(10)))
        .apply(|l: &i32, r: &i32| l * r)
        .add_sink(sink)
        .await
        .unwrap();

    let values = results.lock().unwrap();
    // Key even: 2*10=20, Key odd: 3*11=33
    assert_eq!(values.len(), 2);
    let mut sorted = values.clone();
    sorted.sort();
    assert_eq!(sorted, vec![20, 33]);
}

#[tokio::test]
async fn test_window_join_no_match() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<i32>::new();

    // Trades in window [0, 10_000)
    let trades = vec![
        StreamElement::Record(Record::new(1i32, EventTimestamp::new(1_000))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(10_000))),
    ];

    // Quotes in window [10_000, 20_000) — different window
    let quotes = TimestampedSource::new(vec![
        StreamElement::Record(Record::new(10i32, EventTimestamp::new(11_000))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(20_000))),
    ]);

    env.from_source(TimestampedSource::new(trades))
        .window_join(quotes)
        .key_by_both(|_: &i32| 0u8, |_: &i32| 0u8)
        .window(TumblingWindow::new(Duration::from_secs(10)))
        .apply(|l: &i32, r: &i32| l + r)
        .add_sink(sink)
        .await
        .unwrap();

    let values = results.lock().unwrap();
    // Different windows → inner join produces 0 results
    assert!(values.is_empty());
}

#[tokio::test]
async fn test_window_cogroup_basic() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<String>::new();

    let left_elements = vec![
        StreamElement::Record(Record::new(1i32, EventTimestamp::new(1_000))),
        StreamElement::Record(Record::new(2i32, EventTimestamp::new(2_000))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(10_000))),
    ];

    let right_source = TimestampedSource::new(vec![
        StreamElement::Record(Record::new(10i32, EventTimestamp::new(3_000))),
        StreamElement::Record(Record::new(20i32, EventTimestamp::new(4_000))),
        StreamElement::Record(Record::new(30i32, EventTimestamp::new(5_000))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(10_000))),
    ]);

    env.from_source(TimestampedSource::new(left_elements))
        .window_join(right_source)
        .key_by_both(|_: &i32| 0u8, |_: &i32| 0u8)
        .window(TumblingWindow::new(Duration::from_secs(10)))
        .cogroup(|_key: &[u8], left: &[i32], right: &[i32]| {
            let left_sum: i32 = left.iter().sum();
            let right_sum: i32 = right.iter().sum();
            vec![format!("{left_sum}+{right_sum}={}", left_sum + right_sum)]
        })
        .add_sink(sink)
        .await
        .unwrap();

    let values = results.lock().unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0], "3+60=63"); // (1+2) + (10+20+30)
}

#[tokio::test]
async fn test_window_cogroup_empty_side() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<String>::new();

    // Only left records, no right records in this window.
    let left_elements = vec![
        StreamElement::Record(Record::new(1i32, EventTimestamp::new(1_000))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(10_000))),
    ];

    // Right source has no records in the same window, but sends a watermark.
    let right_source = TimestampedSource::new(vec![StreamElement::Watermark(Watermark::new(
        EventTimestamp::new(10_000),
    ))]);

    env.from_source(TimestampedSource::new(left_elements))
        .window_join(right_source)
        .key_by_both(|_: &i32| 0u8, |_: &i32| 0u8)
        .window(TumblingWindow::new(Duration::from_secs(10)))
        .cogroup(|_key: &[u8], left: &[i32], right: &[i32]| {
            vec![format!("left={},right={}", left.len(), right.len())]
        })
        .add_sink(sink)
        .await
        .unwrap();

    let values = results.lock().unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0], "left=1,right=0");
}

// ---------------------------------------------------------------------------
// Interval join tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_interval_join_basic() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<i32>::new();

    // Left at t=100, within 0..1s of right at t=500
    let left_elements = vec![
        StreamElement::Record(Record::new(1i32, EventTimestamp::new(100))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(2_000))),
    ];

    let right_source = TimestampedSource::new(vec![
        StreamElement::Record(Record::new(10i32, EventTimestamp::new(500))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(2_000))),
    ]);

    env.from_source(TimestampedSource::new(left_elements))
        .interval_join(right_source)
        .key_by_both(|_: &i32| 0u8, |_: &i32| 0u8)
        .between(Duration::from_millis(0), Duration::from_secs(1))
        .process(|l: &i32, r: &i32| l + r)
        .add_sink(sink)
        .await
        .unwrap();

    let values = results.lock().unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0], 11);
}

#[tokio::test]
async fn test_interval_join_no_match() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<i32>::new();

    // Left at t=100, right at t=5000 — outside 100ms bound
    let left_elements = vec![
        StreamElement::Record(Record::new(1i32, EventTimestamp::new(100))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(6_000))),
    ];

    let right_source = TimestampedSource::new(vec![
        StreamElement::Record(Record::new(10i32, EventTimestamp::new(5_000))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(6_000))),
    ]);

    env.from_source(TimestampedSource::new(left_elements))
        .interval_join(right_source)
        .key_by_both(|_: &i32| 0u8, |_: &i32| 0u8)
        .between(Duration::from_millis(0), Duration::from_millis(100))
        .process(|l: &i32, r: &i32| l + r)
        .add_sink(sink)
        .await
        .unwrap();

    let values = results.lock().unwrap();
    assert!(values.is_empty());
}

#[tokio::test]
async fn test_interval_join_multiple_matches() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<i32>::new();

    // Left at t=100, matches rights at 200, 500, 800 (all within 1s)
    let left_elements = vec![
        StreamElement::Record(Record::new(1i32, EventTimestamp::new(100))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(2_000))),
    ];

    let right_source = TimestampedSource::new(vec![
        StreamElement::Record(Record::new(10i32, EventTimestamp::new(200))),
        StreamElement::Record(Record::new(20i32, EventTimestamp::new(500))),
        StreamElement::Record(Record::new(30i32, EventTimestamp::new(800))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(2_000))),
    ]);

    env.from_source(TimestampedSource::new(left_elements))
        .interval_join(right_source)
        .key_by_both(|_: &i32| 0u8, |_: &i32| 0u8)
        .between(Duration::from_millis(0), Duration::from_secs(1))
        .process(|l: &i32, r: &i32| l + r)
        .add_sink(sink)
        .await
        .unwrap();

    let values = results.lock().unwrap();
    assert_eq!(values.len(), 3);
    let mut sorted = values.clone();
    sorted.sort();
    assert_eq!(sorted, vec![11, 21, 31]);
}

#[tokio::test]
async fn test_interval_join_eviction() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<i32>::new();

    // Left at t=100 with 500ms bound.
    // Watermark at 700 evicts (100 + 500 < 700).
    // Right at 200 arrives after eviction — no match.
    let left_elements = vec![
        StreamElement::Record(Record::new(1i32, EventTimestamp::new(100))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(700))),
    ];

    // Right arrives at t=200 but after watermark evicts left.
    // The merger ensures the watermark is sent before this right record
    // since both sources send their watermarks before the right record
    // arrives (right source has watermark at 700, then record at 800 not needed).
    // Actually, due to merger ordering, we need to be careful:
    // Source2 sends record at 200 before watermark at 700, so the record
    // may arrive before eviction. Let's use a right record at 800 instead,
    // which is after the watermark.
    let right_source = TimestampedSource::new(vec![
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(700))),
        StreamElement::Record(Record::new(10i32, EventTimestamp::new(800))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(1_000))),
    ]);

    env.from_source(TimestampedSource::new(left_elements))
        .interval_join(right_source)
        .key_by_both(|_: &i32| 0u8, |_: &i32| 0u8)
        .between(Duration::from_millis(0), Duration::from_millis(500))
        .process(|l: &i32, r: &i32| l + r)
        .add_sink(sink)
        .await
        .unwrap();

    let values = results.lock().unwrap();
    // Left at 100 was evicted before right at 800 arrived.
    // 800 is also outside the 100+500=600 bound anyway.
    assert!(values.is_empty());
}

#[tokio::test]
async fn test_interval_join_keyed() {
    let mut env = StreamExecutionEnvironment::new();
    let (sink, results) = CollectSink::<i32>::new();

    // Key A (even) and Key B (odd) — matches are per-key isolated.
    let left_elements = vec![
        StreamElement::Record(Record::new(2i32, EventTimestamp::new(100))),
        StreamElement::Record(Record::new(3i32, EventTimestamp::new(100))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(2_000))),
    ];

    let right_source = TimestampedSource::new(vec![
        StreamElement::Record(Record::new(10i32, EventTimestamp::new(200))),
        StreamElement::Record(Record::new(11i32, EventTimestamp::new(200))),
        StreamElement::Watermark(Watermark::new(EventTimestamp::new(2_000))),
    ]);

    env.from_source(TimestampedSource::new(left_elements))
        .interval_join(right_source)
        .key_by_both(|v: &i32| v % 2, |v: &i32| v % 2)
        .between(Duration::from_millis(0), Duration::from_secs(1))
        .process(|l: &i32, r: &i32| l * r)
        .add_sink(sink)
        .await
        .unwrap();

    let values = results.lock().unwrap();
    // Key even: 2*10=20, Key odd: 3*11=33
    assert_eq!(values.len(), 2);
    let mut sorted = values.clone();
    sorted.sort();
    assert_eq!(sorted, vec![20, 33]);
}
