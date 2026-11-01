//! Files written by one bounded content transaction. No ACK precedes this barrier.
use std::fs::File;
use std::io;

#[cfg(test)]
thread_local! { static FAIL_NEXT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

#[cfg(test)]
pub(crate) fn fail_next_flush() {
    FAIL_NEXT.set(true);
}

#[derive(Default)]
pub(crate) struct ContentDurability {
    files: Vec<File>,
}

impl ContentDurability {
    pub(crate) fn retain(&mut self, file: File) {
        self.files.push(file);
    }

    pub(crate) fn flush(self) -> io::Result<()> {
        #[cfg(test)]
        let _flushing = crate::store::ingest_profile::span("durability_flush");
        #[cfg(test)]
        if FAIL_NEXT.replace(false) {
            return Err(io::Error::other("injected content durability failure"));
        }
        // Parallel file barriers let the filesystem combine physical flushes.
        // The transaction remains uncommitted and owns every file until all
        // workers return; a failed barrier cannot acknowledge any envelope.
        const WORKERS: usize = 8;
        let mut groups: [Vec<File>; WORKERS] = std::array::from_fn(|_| Vec::new());
        for (index, file) in self.files.into_iter().enumerate() {
            groups[index % WORKERS].push(file);
        }
        std::thread::scope(|scope| {
            let workers: Vec<_> = groups
                .into_iter()
                .filter(|files| !files.is_empty())
                .map(|files| {
                    scope.spawn(move || {
                        for file in files {
                            file.sync_all()?;
                        }
                        Ok::<_, io::Error>(())
                    })
                })
                .collect();
            let mut error = None;
            for worker in workers {
                if let Err(failure) = worker
                    .join()
                    .unwrap_or_else(|_| Err(io::Error::other("content sync worker panicked")))
                {
                    error.get_or_insert(failure);
                }
            }
            error.map_or(Ok(()), Err)
        })
    }
}
