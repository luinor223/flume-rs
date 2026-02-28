//! Two-input merger for connected streams.
//!
//! Reads from two `mpsc::Receiver`s, tags elements as `Either::Left` /
//! `Either::Right`, computes min-watermark, and aligns checkpoint barriers
//! before forwarding into a single output channel.

use flume_core::{
    CheckpointBarrier, Either, EventTimestamp, FlumeError, FlumeResult, Record, StreamElement,
    Watermark,
};
use tokio::sync::mpsc;

/// Merge two input channels into a single `Either`-tagged output channel.
///
/// - Records are tagged and forwarded immediately.
/// - Watermarks: the output watermark is `min(wm1, wm2)`, advancing only
///   when both inputs have progressed.
/// - Barriers: waits for a matching barrier from both inputs before
///   forwarding. Records from the faster input are buffered until the
///   slower input sends its barrier.
/// - When one input closes, the other is drained to completion.
pub async fn two_input_merge<In1, In2>(
    mut input1: mpsc::Receiver<StreamElement<In1>>,
    mut input2: mpsc::Receiver<StreamElement<In2>>,
    output: mpsc::Sender<StreamElement<Either<In1, In2>>>,
) -> FlumeResult<()>
where
    In1: Send + 'static,
    In2: Send + 'static,
{
    let mut watermark1 = EventTimestamp::MIN;
    let mut watermark2 = EventTimestamp::MIN;
    let mut last_forwarded_watermark = EventTimestamp::MIN;

    // Barrier alignment state: when one input sends a barrier, we buffer
    // elements from that input until the other sends its matching barrier.
    let mut pending_barrier_from_1: Option<CheckpointBarrier> = None;
    let mut pending_barrier_from_2: Option<CheckpointBarrier> = None;
    let mut buffer1: Vec<StreamElement<Either<In1, In2>>> = Vec::new();
    let mut buffer2: Vec<StreamElement<Either<In1, In2>>> = Vec::new();

    let mut input1_open = true;
    let mut input2_open = true;

    loop {
        if !input1_open && !input2_open {
            break;
        }

        // If we're waiting for a barrier from input2 (input1 already sent one),
        // only read from input2 (buffer input1 elements).
        if pending_barrier_from_1.is_some() && pending_barrier_from_2.is_none() {
            match input2.recv().await {
                Some(element) => {
                    handle_element2(
                        element,
                        &output,
                        &mut watermark2,
                        &mut last_forwarded_watermark,
                        watermark1,
                        &mut pending_barrier_from_2,
                        &mut buffer2,
                    )
                    .await?;

                    try_flush_barriers(
                        &mut pending_barrier_from_1,
                        &mut pending_barrier_from_2,
                        &mut buffer1,
                        &mut buffer2,
                        &output,
                    )
                    .await?;
                }
                None => {
                    input2_open = false;
                    // If input2 closed while we were waiting for its barrier,
                    // flush what we have.
                    if let Some(barrier) = pending_barrier_from_1.take() {
                        send(&output, StreamElement::CheckpointBarrier(barrier)).await?;
                        flush_buffer(&mut buffer1, &output).await?;
                    }
                }
            }
            continue;
        }

        // If we're waiting for a barrier from input1 (input2 already sent one),
        // only read from input1.
        if pending_barrier_from_2.is_some() && pending_barrier_from_1.is_none() {
            match input1.recv().await {
                Some(element) => {
                    handle_element1(
                        element,
                        &output,
                        &mut watermark1,
                        &mut last_forwarded_watermark,
                        watermark2,
                        &mut pending_barrier_from_1,
                        &mut buffer1,
                    )
                    .await?;

                    try_flush_barriers(
                        &mut pending_barrier_from_1,
                        &mut pending_barrier_from_2,
                        &mut buffer1,
                        &mut buffer2,
                        &output,
                    )
                    .await?;
                }
                None => {
                    input1_open = false;
                    if let Some(barrier) = pending_barrier_from_2.take() {
                        send(&output, StreamElement::CheckpointBarrier(barrier)).await?;
                        flush_buffer(&mut buffer2, &output).await?;
                    }
                }
            }
            continue;
        }

        // Normal mode: read from whichever input has data.
        match (input1_open, input2_open) {
            (true, true) => {
                tokio::select! {
                    result = input1.recv() => {
                        match result {
                            Some(element) => {
                                handle_element1(
                                    element, &output,
                                    &mut watermark1, &mut last_forwarded_watermark, watermark2,
                                    &mut pending_barrier_from_1, &mut buffer1,
                                ).await?;
                            }
                            None => {
                                input1_open = false;
                            }
                        }
                    }
                    result = input2.recv() => {
                        match result {
                            Some(element) => {
                                handle_element2(
                                    element, &output,
                                    &mut watermark2, &mut last_forwarded_watermark, watermark1,
                                    &mut pending_barrier_from_2, &mut buffer2,
                                ).await?;
                            }
                            None => {
                                input2_open = false;
                            }
                        }
                    }
                }
            }
            (true, false) => match input1.recv().await {
                Some(element) => {
                    handle_element1(
                        element,
                        &output,
                        &mut watermark1,
                        &mut last_forwarded_watermark,
                        watermark2,
                        &mut pending_barrier_from_1,
                        &mut buffer1,
                    )
                    .await?;
                }
                None => {
                    input1_open = false;
                }
            },
            (false, true) => match input2.recv().await {
                Some(element) => {
                    handle_element2(
                        element,
                        &output,
                        &mut watermark2,
                        &mut last_forwarded_watermark,
                        watermark1,
                        &mut pending_barrier_from_2,
                        &mut buffer2,
                    )
                    .await?;
                }
                None => {
                    input2_open = false;
                }
            },
            (false, false) => break,
        }
    }

    Ok(())
}

async fn handle_element1<In1: Send, In2: Send>(
    element: StreamElement<In1>,
    output: &mpsc::Sender<StreamElement<Either<In1, In2>>>,
    watermark1: &mut EventTimestamp,
    last_forwarded_watermark: &mut EventTimestamp,
    watermark2: EventTimestamp,
    pending_barrier: &mut Option<CheckpointBarrier>,
    buffer: &mut Vec<StreamElement<Either<In1, In2>>>,
) -> FlumeResult<()> {
    match element {
        StreamElement::Record(record) => {
            let tagged = StreamElement::Record(Record {
                value: Either::Left(record.value),
                timestamp: record.timestamp,
                key: record.key,
            });
            if pending_barrier.is_some() {
                buffer.push(tagged);
            } else {
                send(output, tagged).await?;
            }
        }
        StreamElement::Watermark(wm) => {
            *watermark1 = wm.timestamp;
            let min_wm = std::cmp::min(*watermark1, watermark2);
            if min_wm > *last_forwarded_watermark {
                *last_forwarded_watermark = min_wm;
                send(output, StreamElement::Watermark(Watermark::new(min_wm))).await?;
            }
        }
        StreamElement::CheckpointBarrier(barrier) => {
            *pending_barrier = Some(barrier);
        }
    }
    Ok(())
}

async fn handle_element2<In1: Send, In2: Send>(
    element: StreamElement<In2>,
    output: &mpsc::Sender<StreamElement<Either<In1, In2>>>,
    watermark2: &mut EventTimestamp,
    last_forwarded_watermark: &mut EventTimestamp,
    watermark1: EventTimestamp,
    pending_barrier: &mut Option<CheckpointBarrier>,
    buffer: &mut Vec<StreamElement<Either<In1, In2>>>,
) -> FlumeResult<()> {
    match element {
        StreamElement::Record(record) => {
            let tagged = StreamElement::Record(Record {
                value: Either::Right(record.value),
                timestamp: record.timestamp,
                key: record.key,
            });
            if pending_barrier.is_some() {
                buffer.push(tagged);
            } else {
                send(output, tagged).await?;
            }
        }
        StreamElement::Watermark(wm) => {
            *watermark2 = wm.timestamp;
            let min_wm = std::cmp::min(watermark1, *watermark2);
            if min_wm > *last_forwarded_watermark {
                *last_forwarded_watermark = min_wm;
                send(output, StreamElement::Watermark(Watermark::new(min_wm))).await?;
            }
        }
        StreamElement::CheckpointBarrier(barrier) => {
            *pending_barrier = Some(barrier);
        }
    }
    Ok(())
}

async fn try_flush_barriers<In1: Send, In2: Send>(
    pending1: &mut Option<CheckpointBarrier>,
    pending2: &mut Option<CheckpointBarrier>,
    buffer1: &mut Vec<StreamElement<Either<In1, In2>>>,
    buffer2: &mut Vec<StreamElement<Either<In1, In2>>>,
    output: &mpsc::Sender<StreamElement<Either<In1, In2>>>,
) -> FlumeResult<()> {
    if let (Some(b1), Some(_b2)) = (pending1.as_ref(), pending2.as_ref()) {
        let barrier = *b1;
        *pending1 = None;
        *pending2 = None;
        send(output, StreamElement::CheckpointBarrier(barrier)).await?;
        flush_buffer(buffer1, output).await?;
        flush_buffer(buffer2, output).await?;
    }
    Ok(())
}

async fn flush_buffer<T: Send>(
    buffer: &mut Vec<StreamElement<T>>,
    output: &mpsc::Sender<StreamElement<T>>,
) -> FlumeResult<()> {
    for element in buffer.drain(..) {
        send(output, element).await?;
    }
    Ok(())
}

async fn send<T: Send>(
    output: &mpsc::Sender<StreamElement<T>>,
    element: StreamElement<T>,
) -> FlumeResult<()> {
    output
        .send(element)
        .await
        .map_err(|_| FlumeError::ChannelClosed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_merge_interleaved_records() {
        let (tx1, rx1) = mpsc::channel(16);
        let (tx2, rx2) = mpsc::channel(16);
        let (out_tx, mut out_rx) = mpsc::channel(16);

        // Send records from both inputs, then close.
        tx1.send(StreamElement::Record(Record::new(1, EventTimestamp::new(100))))
            .await
            .unwrap();
        tx2.send(StreamElement::Record(Record::new(
            "a".to_string(),
            EventTimestamp::new(200),
        )))
        .await
        .unwrap();
        tx1.send(StreamElement::Record(Record::new(2, EventTimestamp::new(300))))
            .await
            .unwrap();
        drop(tx1);
        drop(tx2);

        two_input_merge(rx1, rx2, out_tx).await.unwrap();

        let mut records = Vec::new();
        while let Some(element) = out_rx.recv().await {
            if let StreamElement::Record(r) = element {
                records.push(r);
            }
        }

        assert_eq!(records.len(), 3);

        // Check tagging (order may vary due to select!, but all should appear).
        let lefts: Vec<_> = records
            .iter()
            .filter_map(|r| match &r.value {
                Either::Left(v) => Some(*v),
                Either::Right(_) => None,
            })
            .collect();
        let rights: Vec<_> = records
            .iter()
            .filter_map(|r| match &r.value {
                Either::Left(_) => None,
                Either::Right(v) => Some(v.clone()),
            })
            .collect();

        assert_eq!(lefts.len(), 2);
        assert!(lefts.contains(&1));
        assert!(lefts.contains(&2));
        assert_eq!(rights, vec!["a".to_string()]);
    }

    #[tokio::test]
    async fn test_watermark_min_forwarding() {
        let (tx1, rx1) = mpsc::channel::<StreamElement<i32>>(16);
        let (tx2, rx2) = mpsc::channel::<StreamElement<String>>(16);
        let (out_tx, mut out_rx) = mpsc::channel(16);

        // Input1 watermark at 100 — output should NOT forward yet (input2 is still at MIN).
        tx1.send(StreamElement::Watermark(Watermark::new(EventTimestamp::new(100))))
            .await
            .unwrap();

        // Input2 watermark at 50 — output should forward min(100, 50) = 50.
        tx2.send(StreamElement::Watermark(Watermark::new(EventTimestamp::new(50))))
            .await
            .unwrap();

        // Input2 watermark at 200 — output should forward min(100, 200) = 100.
        tx2.send(StreamElement::Watermark(Watermark::new(
            EventTimestamp::new(200),
        )))
        .await
        .unwrap();

        drop(tx1);
        drop(tx2);

        two_input_merge(rx1, rx2, out_tx).await.unwrap();

        let mut watermarks = Vec::new();
        while let Some(element) = out_rx.recv().await {
            if let StreamElement::Watermark(wm) = element {
                watermarks.push(wm.timestamp.as_millis());
            }
        }

        assert_eq!(watermarks, vec![50, 100]);
    }

    #[tokio::test]
    async fn test_barrier_alignment() {
        let (tx1, rx1) = mpsc::channel(16);
        let (tx2, rx2) = mpsc::channel(16);
        let (out_tx, mut out_rx) = mpsc::channel(16);

        let barrier = CheckpointBarrier::new(1, EventTimestamp::new(100));

        // Input1 sends a barrier, then a record (should be buffered).
        tx1.send(StreamElement::CheckpointBarrier(barrier))
            .await
            .unwrap();
        tx1.send(StreamElement::Record(Record::new(
            42,
            EventTimestamp::new(200),
        )))
        .await
        .unwrap();

        // Input2 sends a record, then the matching barrier.
        tx2.send(StreamElement::Record(Record::new(
            "pre".to_string(),
            EventTimestamp::new(150),
        )))
        .await
        .unwrap();
        tx2.send(StreamElement::CheckpointBarrier(barrier))
            .await
            .unwrap();

        drop(tx1);
        drop(tx2);

        two_input_merge(rx1, rx2, out_tx).await.unwrap();

        let mut elements = Vec::new();
        while let Some(element) = out_rx.recv().await {
            elements.push(element);
        }

        // Should see: the pre-barrier record from input2, then the barrier,
        // then the buffered record from input1.
        let barrier_idx = elements
            .iter()
            .position(|e| matches!(e, StreamElement::CheckpointBarrier(_)));
        assert!(barrier_idx.is_some(), "barrier should appear in output");

        let barrier_idx = barrier_idx.unwrap();

        // The record from input2 ("pre") should appear before the barrier.
        let pre_barrier_records: Vec<_> = elements[..barrier_idx]
            .iter()
            .filter_map(|e| match e {
                StreamElement::Record(r) => Some(r),
                _ => None,
            })
            .collect();
        assert!(
            pre_barrier_records
                .iter()
                .any(|r| matches!(&r.value, Either::Right(s) if s == "pre")),
            "input2 record should appear before barrier"
        );

        // The buffered record from input1 should appear after the barrier.
        let post_barrier_records: Vec<_> = elements[barrier_idx + 1..]
            .iter()
            .filter_map(|e| match e {
                StreamElement::Record(r) => Some(r),
                _ => None,
            })
            .collect();
        assert!(
            post_barrier_records
                .iter()
                .any(|r| matches!(&r.value, Either::Left(42))),
            "input1 record should appear after barrier"
        );
    }

    #[tokio::test]
    async fn test_one_input_closes_early() {
        let (tx1, rx1) = mpsc::channel::<StreamElement<i32>>(16);
        let (tx2, rx2) = mpsc::channel(16);
        let (out_tx, mut out_rx) = mpsc::channel(16);

        // Close input1 immediately.
        drop(tx1);

        // Input2 sends records.
        tx2.send(StreamElement::Record(Record::new(
            "a".to_string(),
            EventTimestamp::new(100),
        )))
        .await
        .unwrap();
        tx2.send(StreamElement::Record(Record::new(
            "b".to_string(),
            EventTimestamp::new(200),
        )))
        .await
        .unwrap();
        drop(tx2);

        two_input_merge(rx1, rx2, out_tx).await.unwrap();

        let mut records = Vec::new();
        while let Some(element) = out_rx.recv().await {
            if let StreamElement::Record(r) = element {
                records.push(r);
            }
        }

        assert_eq!(records.len(), 2);
        assert!(matches!(&records[0].value, Either::Right(s) if s == "a"));
        assert!(matches!(&records[1].value, Either::Right(s) if s == "b"));
    }
}
