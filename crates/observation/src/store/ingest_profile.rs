//! Test-only decomposition of real content persistence, without recording content.
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

thread_local! {
    static SAMPLES: RefCell<Option<BTreeMap<&'static str, (u64, Duration)>>> = const { RefCell::new(None) };
}

pub(crate) struct Span(&'static str, Instant);

pub(crate) fn span(stage: &'static str) -> Span {
    Span(stage, Instant::now())
}

impl Drop for Span {
    fn drop(&mut self) {
        SAMPLES.with_borrow_mut(|samples| {
            if let Some(samples) = samples {
                let entry = samples.entry(self.0).or_default();
                entry.0 += 1;
                entry.1 += self.1.elapsed();
            }
        });
    }
}

pub(crate) fn begin() {
    SAMPLES.with_borrow_mut(|samples| *samples = Some(BTreeMap::new()));
}

pub(crate) fn finish() -> BTreeMap<&'static str, (u64, u128)> {
    SAMPLES.with_borrow_mut(|samples| {
        samples
            .take()
            .unwrap_or_default()
            .into_iter()
            .map(|(stage, (count, duration))| (stage, (count, duration.as_micros())))
            .collect()
    })
}
