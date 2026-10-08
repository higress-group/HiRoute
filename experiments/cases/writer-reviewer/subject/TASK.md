# Editorial assignment

As of October 6, 2026, write an original English explanatory feature for curious
general readers about how improved AI theorem proving changes what mathematics
is for, how people learn it, and what its institutions reward.

Use only MATERIALS.md and this project's outputs as your factual sources. The
packet contains research notes, not prose to imitate. Choose your own title,
argument, structure and imagery. Do not claim firsthand reporting, invent direct
quotes, fabricate details, or turn a source's fears or company claims into
established facts. Keep mathematical scope and uncertainty precise. You may
explain ideas using a clearly hypothetical example or your own analogy.

Aim for 1,200–1,600 words excluding the source list. Give the reader a clear reason
to care; explain rather than merely list opinions. Include a serious alternative
view and a concrete account of what could change in practice. Use brief inline
source markers such as [S2] and a source list. You need not use every supplied
fact. No web browsing, external research or additional interviews.

Work in four stages when requested. Write draft.md, review.json, final.md and
final-review.json at their respective stages; never overwrite earlier artifacts.
Use local tools to read and save files. The final article is judged independently;
the final review is a diagnostic artifact and cannot trigger another revision.

A review must be a JSON object with `verdict` (`publish`, `revise`, or `reject`),
`summary` (string), and `findings` (list). Each finding has `severity` (`critical`,
`major`, or `minor`), `location` (specific article phrase or paragraph), `problem`,
`evidence` (source ID or explanation), and `suggested_change`. Reviews should be
specific and proportionate; an empty list is allowed when justified. Do not
rewrite the article as part of a review.
