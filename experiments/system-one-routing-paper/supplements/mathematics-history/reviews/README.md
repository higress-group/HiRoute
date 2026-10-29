# Labeling evidence

Final scoring labels are in `../datasets/`. The files here preserve earlier
positions and the available review evidence; they are not alternative gold.

- `confirmation-author-labels.jsonl` retains the independent author's original
  136 labels, evidence locations and pairing fields. The normalized scoring labels
  map `bounded/open_reasoning` to `routine/reasoning` and `regular/enhanced` to
  `economy/primary`; applicability states map by meaning. All 136 actions remain
  determinate. `confirmation-author-audit.json` records that author's self-review,
  including a second pass, not review by a separate blinded author or human expert.
- `anonymous-labeling-brief.md` is the historical instruction supplied to the
  92-record independent AI reviewer. Its source-only filenames describe the
  original blind sequence. The same anonymous inputs are now split between
  `../datasets/targeted-inputs.jsonl` and `../datasets/chronological-inputs.jsonl`;
  IDs are unchanged. Scoring labels were not shown during initial review.
- `extension-initial-labels.jsonl` preserves all initial judgments byte for byte;
  its SHA-256 is recorded in `extension-review-summary.json`. The reviewer froze
  these judgments before inspecting author answers or source mapping.
- `extension-author-labels.jsonl` preserves the original 92 author labels.
  `extension-adjudication.jsonl` records semantic comparisons, source/pair IDs and
  corrected evidence locations. `extension-review-summary.json` records resulting
  coverage and interpretation boundaries. The 48 targeted key labels agreed;
  chronological final labels contain 32 determinate, eight conditional and four
  insufficient actions. These positions were finalized before evaluated calls.

The brief and initial rationales are historical evidence. Any initial language
about a missing threshold should be read with the explicit clarification in the
summary: the threshold was fixed at `h < 1`; the unresolved fact was the relation
of continuous `h` to that threshold for a modal/ordinal grade-one case. This is
not permission to infer an exact action from a categorical grade.

The chronological author's old `assessment_grade` denotes failure evidence,
not product performance: 0 means no failure, 1 uncertainty, 2 visible failure.
The separate `stage_grade` uses the performance anchors 0/1/2. Evidence paths in
old author labels use the original source schema; adjudication preserves the
corresponding paths in the normalized public conversation.

This release exposes label disagreements and uncertainty while omitting private
locations and model identities. File hashes can verify published bytes. They do
not independently prove the historical timing of blind access, and procedural
separation is not a sandbox guarantee. None of these follow-ups was validated by
an expert human annotator.
