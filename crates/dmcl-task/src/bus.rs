//! 可增长的事件总线。
//!
//! 事件按订阅者的 cursor 保留，慢订阅者不会丢事件；所有订阅者都消费过的前缀会被裁掉。
//! 没有订阅者时不保留事件。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::watch;

use crate::event::TaskEvent;

struct Subscriber {
    id: u64,
    cursor: u64,
}

struct BusInner {
    events: VecDeque<TaskEvent>,
    /// `events[0]` 的绝对序号。
    base: u64,
    /// 已发布事件总数，也是下一个事件的绝对序号。
    published: u64,
    next_id: u64,
    subscribers: Vec<Subscriber>,
}

impl BusInner {
    fn trim(&mut self) {
        if self.subscribers.is_empty() {
            self.events.clear();
            self.base = self.published;
            return;
        }
        let min = self
            .subscribers
            .iter()
            .map(|subscriber| subscriber.cursor)
            .min()
            .unwrap_or(self.published);
        while self.base < min {
            self.events.pop_front();
            self.base += 1;
        }
    }
}

pub(crate) struct EventBus {
    inner: Mutex<BusInner>,
    closed: AtomicBool,
    wake: watch::Sender<u64>,
}

impl EventBus {
    pub(crate) fn new() -> Arc<Self> {
        let (wake, _) = watch::channel(0);
        Arc::new(Self {
            inner: Mutex::new(BusInner {
                events: VecDeque::new(),
                base: 0,
                published: 0,
                next_id: 0,
                subscribers: Vec::new(),
            }),
            closed: AtomicBool::new(false),
            wake,
        })
    }

    fn lock(&self) -> MutexGuard<'_, BusInner> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn subscribe(self: &Arc<Self>) -> EventReceiver {
        let mut inner = self.lock();
        let id = inner.next_id;
        inner.next_id += 1;
        let cursor = inner.published;
        inner.subscribers.push(Subscriber { id, cursor });
        drop(inner);
        EventReceiver {
            bus: self.clone(),
            id,
            cursor,
        }
    }

    /// 发布一条事件；返回是否至少有一个订阅者会收到它。
    pub(crate) fn publish(&self, event: TaskEvent) -> bool {
        let mut inner = self.lock();
        if inner.subscribers.is_empty() {
            return false;
        }
        let sequence = inner.published;
        inner.published += 1;
        inner.events.push_back(event);
        inner.trim();
        drop(inner);
        let _ = self.wake.send(sequence + 1);
        true
    }

    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let published = self.lock().published;
        let _ = self.wake.send(published);
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn unsubscribe(&self, id: u64) {
        let mut inner = self.lock();
        inner.subscribers.retain(|subscriber| subscriber.id != id);
        inner.trim();
    }

    fn fetch(&self, id: u64, cursor: &mut u64) -> Option<TaskEvent> {
        let mut inner = self.lock();
        if *cursor >= inner.published {
            return None;
        }
        let index = (*cursor).checked_sub(inner.base)? as usize;
        let event = inner.events.get(index)?.clone();
        *cursor += 1;
        if let Some(subscriber) = inner
            .subscribers
            .iter_mut()
            .find(|subscriber| subscriber.id == id)
        {
            subscriber.cursor = *cursor;
        }
        inner.trim();
        Some(event)
    }
}

/// 事件总线上的一个订阅者。
pub struct EventReceiver {
    bus: Arc<EventBus>,
    id: u64,
    cursor: u64,
}

impl EventReceiver {
    /// 等待下一条事件；总线关闭且事件排空后返回 `None`。
    pub async fn recv(&mut self) -> Option<TaskEvent> {
        let mut wake = self.bus.wake.subscribe();
        loop {
            if let Some(event) = self.try_recv() {
                return Some(event);
            }
            if self.bus.is_closed() {
                return None;
            }
            // 发布事件或关闭总线都会唤醒；即使通知先于 `changed()` 到达，
            // watch 也会因为值未被标记为已读而立即返回。
            let _ = wake.changed().await;
        }
    }

    /// 不等待，取一条已经发布的事件。
    pub fn try_recv(&mut self) -> Option<TaskEvent> {
        self.bus.fetch(self.id, &mut self.cursor)
    }
}

impl Drop for EventReceiver {
    fn drop(&mut self) {
        self.bus.unsubscribe(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(message: &str) -> TaskEvent {
        TaskEvent::Log {
            task: None,
            message: message.to_string(),
        }
    }

    #[tokio::test]
    async fn a_slow_subscriber_receives_every_event() {
        let bus = EventBus::new();
        let mut slow = bus.subscribe();
        let mut fast = bus.subscribe();

        for index in 0..10_000 {
            bus.publish(log(&index.to_string()));
        }
        bus.close();

        let mut fast_seen = 0;
        while fast.try_recv().is_some() {
            fast_seen += 1;
        }
        assert_eq!(fast_seen, 10_000);

        let mut slow_seen = 0;
        while let Some(event) = slow.recv().await {
            if let TaskEvent::Log { message, .. } = event {
                assert_eq!(message, slow_seen.to_string());
            }
            slow_seen += 1;
        }
        assert_eq!(slow_seen, 10_000);
    }

    #[tokio::test]
    async fn subscribers_are_independent() {
        let bus = EventBus::new();
        let mut first = bus.subscribe();
        let mut second = bus.subscribe();

        bus.publish(log("a"));
        bus.publish(log("b"));
        bus.close();

        assert!(first.try_recv().is_some());
        assert!(first.try_recv().is_some());
        assert!(first.try_recv().is_none());
        assert!(second.try_recv().is_some());
        assert!(second.try_recv().is_some());
        assert!(second.try_recv().is_none());
    }

    #[tokio::test]
    async fn events_are_not_retained_without_subscribers() {
        let bus = EventBus::new();
        assert!(!bus.publish(log("dropped")));
        let mut late = bus.subscribe();
        assert!(late.try_recv().is_none());
        bus.close();
        assert!(late.recv().await.is_none());
    }

    #[tokio::test]
    async fn recv_wakes_up_on_publish_and_close() {
        let bus = EventBus::new();
        let mut receiver = bus.subscribe();

        let publisher = {
            let bus = bus.clone();
            tokio::spawn(async move {
                tokio::task::yield_now().await;
                bus.publish(log("hello"));
                bus.close();
            })
        };

        let event = receiver.recv().await.expect("should receive the event");
        assert!(matches!(event, TaskEvent::Log { .. }));
        assert!(receiver.recv().await.is_none());
        publisher.await.unwrap();
    }
}
