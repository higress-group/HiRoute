# Release documentation review

Keep one completed `<tag>.json` here for each new product release, in the same
website PR as its release catalog entry. The product SHA is the exact frozen
package source; the website commit can be later. Follow the
[documentation maintenance policy](../../README.md#documentation-maintenance)
before creating the record. No review is required merely to edit a guide.

1. Review the product changes since the preceding release. Update affected Chinese
   and English guides, copied Agent/CLI instructions, navigation and captions.
2. Inspect affected screenshots against the candidate UI. Recapture changed flows
   in dark mode, inspect both languages and update provenance. Reuse unchanged
   captures only with a concrete explanation of why they still match.
3. After the content is ready, generate a **pending** record from the repository
   root, replacing the example tag and `FULL_CANDIDATE_SHA`:

   ```sh
   node apps/website/scripts/documentation-review.mjs prepare --tag v1.2.3 --revision FULL_CANDIDATE_SHA > apps/website/docs/release-reviews/v1.2.3.json
   ```

4. Fill both `guides` and `screenshots`: choose `updated` or `unchanged`, write a
   `summary` of the reviewed impact and result, and list the relevant
   repository-relative `reviewed_files`. For example, name both localized guide
   sources and both localized captures. An unchanged verdict needs an explanation;
   do not update the digest or status merely to silence a failing check.
5. Check the completed record and website before publishing the GitHub Release:

   ```sh
   node apps/website/scripts/documentation-review.mjs check --tag v1.2.3 --revision FULL_CANDIDATE_SHA
   npm --prefix apps/website test
   npm --prefix apps/website run build
   ```

The helper's `content_sha256` fields cover file names and bytes, so later edits,
added pages, replaced/deleted images or a changed copy interaction invalidate the
review. Reinspect the changes before preparing and completing a replacement.
The guide digest covers both root READMEs, website guide sources/rendering/scripts
and canonical decision documentation/API JSON. The screenshot digest covers
`public/guide-assets`, canonical decision illustrations and their provenance.
Generated files and the release review itself are excluded.

`check-all` validates unrecorded releases during ordinary website checks. The
publication workflow additionally selects the actual tag/source and requires its
review before downloading or uploading release assets. Existing recorded releases
can be republished at their exact historical SHA without rolling today's guides
back to an old UI. A different source under the same tag requires a new review;
other release qualification and same-version replacement rules still apply.

These records are documentation assessments, not package/native acceptance,
publication approval or a replacement for the existing release catalog.
