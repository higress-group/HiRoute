# Model reference fragments (MVP-10)

`ReferenceDetails` consumes the shared Application rating result for the selected exact native configuration. It does not select, sort, or establish source capability. `PriceEditor` uses the native `effective_price_query` and `preview_price_change` commands; the backend-held confirmation, rendered by the shared WebView host, registers the price-specific protected grant and reuses exact operation recovery.

Manual pricing edits input, output, cache-read and cache-write rates. Blank cache rates remain unknown; zero requires explicit input. Key spending limits require a complete same-currency usage-estimate quote, so this editor also repairs missing rates without inventing provider billing conditions.

MVP-11 owns mounting these fragments in its selected-source model page. Pass its source/binding/override revisions, source name and trusted writable state. Mount `PriceEditor` with a key containing source identity, model identity, currency and valuation kind. `onRefresh` must reload those revisions from the backend; `onOperation` must join the existing shared operation observation UI. A read failure retains drafts. Decimal rate input and displayed rates remain strings across WebView IPC, including values above JS's safe integer range.

The parent supplies the selected rating result and its actual snapshot reference; never combine a result with another snapshot's labels. The standalone fragments do not constitute installed Desktop E2E evidence or the complete model page.
