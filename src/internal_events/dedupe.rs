use metrics::counter;
use vector_core::internal_event::InternalEvent;

#[derive(Debug)]
pub struct DedupeEventsDropped<const N: usize> {
    pub events: [crate::event::Event; N],
}

impl<const N: usize> InternalEvent for DedupeEventsDropped<N> {
    fn emit(self) {
        debug!(
            message = "Events dropped.",
            count = self.events.len(),
            intentional = true,
            reason = "Events have been found in cache for deduplication."
        );
        counter!("events_discarded_total", self.events.len() as u64);
    }
}
