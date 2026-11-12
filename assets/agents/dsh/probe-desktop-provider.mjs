import { join } from "node:path";
import { pathToFileURL } from "node:url";

const runtime = pathToFileURL(join(process.argv[1], "dsh/package.json")).href;
const modules = [
  ["@deepseek-ai/dsh-llm-pi-ai", "PiAiAdapter"],
  ["@earendil-works/pi-ai/api/openai-responses.lazy", "openAIResponsesApi"],
  ["@earendil-works/pi-ai/api/anthropic-messages.lazy", "anthropicMessagesApi"],
];
try {
  for (const [name, member] of modules) {
    const exports = await import(import.meta.resolve(name, runtime));
    if (typeof exports[member] !== "function") process.exit(1);
  }
  console.log("HIRoute Desktop provider ready");
} catch {
  process.exit(1);
}
