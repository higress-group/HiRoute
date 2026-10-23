# Current client-bundled ReleaseFacts

This is the only production ReleaseFacts catalog in the MVP. It is generated with the client and
embedded directly into the daemon build. Production neither installs nor reads a second copy under
the mutable storage root. A later client revision replaces these build inputs in place; there is no
production generation selector, detached signature, release private key, signer approval, or older
production fallback.

Version suffixes that remain inside JSON schema names, adapter IDs, or HTTP paths describe wire
contract revisions only. They do not identify selectable model-catalog editions.

Generate and verify the checked-in bytes twice:

```sh
python3 assets/release-facts/current/prepare-bundle.py
python3 assets/release-facts/current/prepare-bundle.py --check
cargo run -p hiroute-release-facts -- check \
  --input assets/release-facts/current/compiler-input.json \
  --output-dir assets/release-facts/current/bundle
```

`compiler-input.json` is the reviewable compiler input. `bundle/` contains the exact manifest,
connector registry, model data, and agent profiles embedded in the client. Production pins the
exact manifest bytes in the same build and then validates the registry/model-data byte digests,
product identity, and cross-reference digest before exposing any catalog fact. Missing, stale, or
modified bytes fail closed.

`compiler-input.json` includes only the additional documented pricing facts, public connection hints
and semantic revision beside the shared model/rating/metadata input. Both compilers emit the same
`bundle/official-model-data.json` from those inputs for
both the embedded fallback and the [complete HTTPS feed](../../model-feed/README.md). A selected
verified website/cache snapshot can replace descriptive model and default-price facts as a whole.
The installed registry, authentication, adapters and free-offer eligibility remain release-owned.
Semantic data revisions/digests and publication sequences have independent acceptance floors.
