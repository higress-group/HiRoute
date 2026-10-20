//! Retrieval and candidate preparation never hold the feed or Control writer lock.
use super::*;

struct PreparedPublication<P> {
    validated: ValidatedFeed,
    bytes: Vec<u8>,
    etag: Option<String>,
    catalog: ReleaseCatalogSnapshot,
    counts: FeedCounts,
    price: Option<P>,
}
impl ModelFeedService {
    pub fn refresh_prepared<P>(
        &self,
        prepare: impl FnOnce(&ReleaseCatalogSnapshot) -> Result<P, String>,
        install: impl FnOnce(
            &ReleaseCatalogSnapshot,
            P,
            &mut dyn FnMut() -> Result<(), String>,
        ) -> Result<(), String>,
    ) -> Result<(), String> {
        #[cfg(feature = "integration-test-hooks")]
        if let Some(root) = product_fixture::root() {
            return self.refresh_using_prepared(
                |_| product_fixture::download(&root),
                prepare,
                install,
            );
        }
        self.refresh_using_prepared(
            |etag| retrieve_feed(&self.client_version, etag),
            prepare,
            install,
        )
    }
    #[cfg(test)]
    pub(super) fn refresh_using(
        &self,
        retrieve: impl FnMut(
            Option<&str>,
        )
            -> Result<FeedDownload, hiroute_integrations::model_feed::FeedError>,
        install: impl FnOnce(u64, u64) -> Result<(), String>,
    ) -> Result<(), String> {
        self.refresh_using_prepared(
            retrieve,
            |_| Ok(()),
            |catalog, (), commit| {
                commit()?;
                install(catalog.generation, catalog.accepted_sequence)
            },
        )
    }
    pub(super) fn refresh_using_prepared<P>(
        &self,
        mut retrieve: impl FnMut(
            Option<&str>,
        )
            -> Result<FeedDownload, hiroute_integrations::model_feed::FeedError>,
        prepare: impl FnOnce(&ReleaseCatalogSnapshot) -> Result<P, String>,
        install: impl FnOnce(
            &ReleaseCatalogSnapshot,
            P,
            &mut dyn FnMut() -> Result<(), String>,
        ) -> Result<(), String>,
    ) -> Result<(), String> {
        let (generation, epoch, selected, etag) = {
            let mut guard = self.mutable.lock().map_err(|_| "feed_lock")?;
            check_mutable(&guard)?;
            if guard.refreshing {
                while guard.refreshing {
                    guard = self.refresh_finished.wait(guard).map_err(|_| "feed_lock")?;
                }
                return guard.last_refresh_result.clone().unwrap_or(Ok(()));
            }
            guard.refreshing = true;
            (
                guard.state.generation,
                guard.retrieval_epoch,
                guard.state.selected_data.clone(),
                guard.state.held.as_ref().and(guard.state.etag.clone()),
            )
        };
        let fetched = retrieve(etag.as_deref())
            .and_then(|value| {
                if matches!(value, FeedDownload::NotModified) && etag.is_none() {
                    retrieve(None)
                } else {
                    Ok(value)
                }
            })
            .map_err(|e| failure_code(e.kind).to_owned());
        // Even large snapshots are parsed, qualified and price-prepared before either writer lock.
        let prepared = fetched.and_then(|download| match download {
            FeedDownload::Content { bytes, etag } => {
                let validated = parse_feed(&bytes).map_err(|e| failure_code(e.kind).to_owned())?;
                let incoming = DataFloor {
                    revision: validated.payload.data_revision,
                    digest: validated.payload.data_digest.clone(),
                };
                let semantic_change = selected.as_ref() != Some(&incoming);
                let admission = admit(&self.embedded, &validated.payload, &self.client_version)
                    .map_err(|e| e.to_string())?;
                let catalog = ReleaseCatalogSnapshot {
                    catalog: self
                        .embedded
                        .with_feed(&admission)
                        .map_err(|e| e.to_string())?,
                    generation: if semantic_change {
                        generation
                            .checked_add(1)
                            .ok_or("feed_generation_overflow")?
                    } else {
                        generation
                    },
                    accepted_sequence: validated.payload.sequence,
                };
                let price = if semantic_change {
                    Some(prepare(&catalog)?)
                } else {
                    None
                };
                Ok((
                    Some(PreparedPublication {
                        validated,
                        bytes,
                        etag,
                        catalog,
                        counts: admission.counts,
                        price,
                    }),
                    "ok",
                ))
            }
            FeedDownload::NotModified => Ok((None, "not_modified")),
            FeedDownload::NotPublished => Ok((None, "not_published")),
        });
        let mut guard = self.mutable.lock().map_err(|_| "feed_lock")?;
        guard.refreshing = false;
        let result = if generation != guard.state.generation || epoch != guard.retrieval_epoch {
            Err("feed_retrieval_superseded".into())
        } else {
            prepared.and_then(|(prepared, code)| match prepared {
                Some(prepared) => self.apply_prepared(&mut guard, prepared, install),
                None if code == "not_modified" && guard.state.held.is_none() => {
                    Err("cacheless_not_modified".into())
                }
                None => Ok(code),
            })
        };
        let (code, reason) = match &result {
            Ok(code) => ((*code).to_owned(), None),
            Err(reason) => (attempt_failure_code(reason).into(), Some(reason.clone())),
        };
        guard.state.last_attempt = Some(FeedAttempt {
            at_ms: now_ms(),
            result: code,
            reason,
        });
        guard.failures = if result.is_ok() {
            0
        } else {
            guard.failures.saturating_add(1)
        };
        let delay = if guard.failures == 0 {
            330 * 60 * 1000 + jitter_ms(60 * 60 * 1000)
        } else {
            (15 * 60 * 1000_u64).saturating_mul(1 << guard.failures.min(4).saturating_sub(1))
        };
        guard.next_attempt = guard.state.retrieval_enabled.then(|| now_ms() + delay);
        let result = self.store.commit(&guard.state).and(result.map(|_| ()));
        guard.last_refresh_result = Some(result.clone());
        self.refresh_finished.notify_all();
        result
    }
    fn apply_prepared<P>(
        &self,
        guard: &mut MutableFeed,
        prepared: PreparedPublication<P>,
        install: impl FnOnce(
            &ReleaseCatalogSnapshot,
            P,
            &mut dyn FnMut() -> Result<(), String>,
        ) -> Result<(), String>,
    ) -> Result<&'static str, String> {
        let PreparedPublication {
            validated,
            bytes,
            etag,
            catalog,
            counts,
            price,
        } = prepared;
        if validated.payload.sequence <= guard.state.sequence_floor
            && !guard.state.highest_accepted.as_ref().is_some_and(|h| {
                h.sequence >= guard.state.sequence_floor && h.sequence == validated.payload.sequence
            })
        {
            return Err("older_sequence".into());
        }
        if let Some(high) = &guard.state.highest_accepted {
            if validated.payload.sequence < high.sequence {
                return Err("older_sequence".into());
            }
            if validated.payload.sequence == high.sequence {
                if validated.payload_digest != high.payload_digest {
                    return Err("same_sequence_conflicting_digest".into());
                }
                if guard.state.held.is_some() {
                    return Ok("already_current");
                }
            }
        }
        let incoming = DataFloor {
            revision: validated.payload.data_revision,
            digest: validated.payload.data_digest.clone(),
        };
        if let Some(floor) = &guard.state.data_floor {
            if incoming.revision < floor.revision {
                return Err("older_data_revision".into());
            }
            if incoming.revision == floor.revision && incoming.digest != floor.digest {
                return Err("same_data_revision_conflicting_digest".into());
            }
        }
        let mut next = guard.state.clone();
        next.generation = catalog.generation;
        next.data_floor = Some(incoming.clone());
        next.selected_data = Some(incoming);
        next.highest_accepted = Some(HighestAccepted {
            sequence: validated.payload.sequence,
            payload_digest: validated.payload_digest.clone(),
        });
        next.held = Some(HeldPublication {
            sequence: validated.payload.sequence,
            payload_digest: validated.payload_digest.clone(),
            retrieved_at_ms: now_ms(),
            issued_at: validated.payload.issued_at,
        });
        next.etag = etag;
        next.last_success_at_ms = Some(now_ms());
        // Content-addressed bytes grant no authority until the checkpoint/state is committed.
        self.store.persist_blob(&bytes, &validated.payload_digest)?;
        let mut committed = false;
        self.active.store(false, Ordering::Release);
        let result = if let Some(price) = price {
            let mut commit = || {
                if committed {
                    return Err("feed_duplicate_commit".into());
                }
                self.store.commit(&next)?;
                committed = true;
                Ok(())
            };
            install(&catalog, price, &mut commit)
        } else {
            self.store.commit(&next).map(|()| {
                committed = true;
            })
        };
        if let Err(error) = result {
            if committed {
                guard.state = next;
                guard.counts = counts;
                guard.refused = Some("feed_activation_requires_restart".into());
                guard.recovery_available = false;
            }
            self.active.store(true, Ordering::Release);
            return Err(error);
        }
        if !committed {
            self.active.store(true, Ordering::Release);
            return Err("feed_missing_commit".into());
        }
        guard.state = next;
        guard.counts = counts;
        self.current.store(Arc::new(catalog));
        self.active.store(true, Ordering::Release);
        self.store.retire_unreferenced(guard.state.held.as_ref());
        Ok("ok")
    }
}
