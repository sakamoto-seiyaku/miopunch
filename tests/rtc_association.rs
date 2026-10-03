use miopunch_native_probe::rtc_association::{DatagramQueue, QueueError};

#[test]
fn full_queue_refuses_more_without_accepting_or_losing_messages() {
    let queue = DatagramQueue::new(2400, 1200);
    queue.open().unwrap();
    queue.try_push(vec![1; 1200]).unwrap();
    queue.try_push(vec![2; 1200]).unwrap();
    assert_eq!(queue.try_push(vec![3]), Err(QueueError::Full));
    assert_eq!(queue.bytes(), 2400);
    assert_eq!(queue.try_pop().unwrap(), Some(vec![1; 1200]));
    assert_eq!(queue.try_pop().unwrap(), Some(vec![2; 1200]));
    assert_eq!(queue.peak_bytes(), 2400);
}

#[test]
fn close_wakes_readers_and_full_senders_and_late_open_is_terminal() {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        task::{Context, Wake, Waker},
    };
    struct Counter(AtomicUsize);
    impl Wake for Counter {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let count = Arc::new(Counter(AtomicUsize::new(0)));
    let waker = Waker::from(count.clone());
    let mut cx = Context::from_waker(&waker);
    let tx = DatagramQueue::new(1200, 1200);
    tx.open().unwrap();
    tx.try_push(vec![1; 1200]).unwrap();
    let rx = DatagramQueue::new(1200, 1200);
    rx.open().unwrap();
    assert!(tx.poll_capacity(&mut cx, 1200).is_pending());
    assert!(rx.poll_pop(&mut cx).is_pending());
    tx.close();
    rx.close();
    assert_eq!(count.0.load(Ordering::SeqCst), 2);
    assert_eq!(tx.open(), Err(QueueError::Closed));
    assert_eq!(tx.try_push(vec![2; 1200]), Err(QueueError::Closed));
    assert_eq!(tx.bytes(), 0);
    assert_eq!(
        rx.poll_pop(&mut cx),
        std::task::Poll::Ready(Err(QueueError::Closed))
    );
    assert_eq!(
        tx.poll_capacity(&mut cx, 1200),
        std::task::Poll::Ready(Err(QueueError::Closed))
    );
}

#[test]
fn small_messages_cannot_bypass_queue_memory_budget() {
    let queue = DatagramQueue::new(2400, 1200);
    queue.open().unwrap();
    queue.try_push(vec![1]).unwrap();
    queue.try_push(vec![2]).unwrap();
    assert_eq!(queue.try_push(vec![3]), Err(QueueError::Full));
    assert_eq!(queue.peak_messages(), 2);
    assert_eq!(queue.try_pop().unwrap(), Some(vec![1]));
    queue.try_push(vec![3]).unwrap();
}
