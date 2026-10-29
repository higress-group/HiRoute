# Responses commentary history from the installed DSH producer

`replayed-input.json` is entirely synthetic test data. It was emitted offline by
DSH `@deepseek-ai/dsh-llm-pi-ai` and `@deepseek-ai/dsh-session` 0.2.0-rc.2 with
`@earendil-works/pi-ai` 0.87.1. The fixture records SHA-256 of each used source file.
It is not a capture of an original HTTP request or a paid provider/native-UI run.

The installed pi-ai `processResponsesStream` consumed controlled Responses lifecycle
events containing commentary, two named function calls and a final answer. The
installed DSH `toPiReplayState` retained the resulting metadata with synthetic
Harness content and tool results. A JSON serialization/reload of those native session
events preceded installed `foldSurface`, `deriveEventMessage`, `foldRequestHeader`,
`textOnlyContext`, `normalizeContext`, and `convertResponsesMessages`.
Only import addresses and exports exposing the two pure DSH functions were added in
memory; installed source and function bodies were unchanged. Fetch and socket-connect
were disabled; network calls and replay degradation were both zero.

The listener scenario consumes this producer's commentary/tool/result/final sequence,
checks it against the controlled upstream output, then verifies actual execution by
a Chat candidate, including after Gateway restart. The fixture preserves the client's
native choices: message IDs/phases/status remain, function-call `fc_` item IDs remain,
and the client omits function-call status. HiRoute must preserve those supplied fields
for Responses and derive only candidate-local Chat history. Consecutive assistant
items are legal Chat history; real provider acceptance remains a separate gate.

Regenerate only with an explicitly reviewed producer version and retained source
hashes. Keep the original fixture when a newly supported producer has different
semantics; a current serializer must not silently replace this producer evidence.
