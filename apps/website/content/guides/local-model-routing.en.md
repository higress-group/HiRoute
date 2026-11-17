# Combine local and frontier models, with local decisions

Use a local Qwen3.8-27B for routine work, a cloud frontier model for harder turns, and a locally served D3 decision model to judge difficulty and the preceding stage. HiRoute connects these three roles to one route that you can select in OpenCode or another supported Agent.

This guide describes a feasible integration based on HiRoute's interfaces and upstream documentation. The complete Qwen + D3 setup has not been tested end to end by HiRoute; routing quality, latency and hardware capacity still need evaluation on the deployment you choose.

<aside class="agent-guide" id="agent-cli" aria-label="Instructions for an Agent">
<div class="agent-guide-heading"><strong>Let an Agent follow this guide</strong></div>
<details class="agent-guide-preview">
<summary>Preview the instructions</summary>

```text
Read https://hiroute.ai/en/docs/local-model-routing/ and help me plan or configure local Qwen3.8-27B execution, a local vLLM Semantic Router D3 decision endpoint, and a cloud primary model according to my goal. Distinguish protocol feasibility from actual inference evidence. Reuse existing services; do not download model weights or start inference servers unless I request deployment and the target has sufficient capacity.
CLI reference: https://hiroute.ai/en/docs/cli/
Use the built-in management Skill: https://hiroute.ai/en/docs/#management-skill
- Inspect execution models and add an API: compute list · compute connection prepare / test / preview / apply
- Inspect and configure the System One connection: decision connections options / list / apply / test
- Configure difficulty routing and connect the Agent: routing preview / apply · agents connect preview / apply
- Review model switches and costs: sessions timeline · prices effective · prices override preview / apply
The UI and CLI share configuration and records. Inspect the current setup first and follow installed --help and hiroute schema show --command-id COMMAND_ID --include-schema. Resolve addresses from the machine running the HiRoute service. Use the actual served model ID, authentication and context limit. Never describe a connection test as proof of judgment quality.
```

</details>
</aside>

## The three roles

| Role | Example | Connection in HiRoute |
| --- | --- | --- |
| Regular execution | Local `Qwen/Qwen3.8-27B`, served as `qwen3.8:27b` by Ollama | General model, OpenAI Chat Completions |
| Primary execution | A frontier model from an API or subscription you already connected | General model, compatible with your Agent's requests |
| Decision | Local `vllm-sr/d3` | Decision model, System One |

Qwen's official model ID is [Qwen/Qwen3.8-27B](https://huggingface.co/Qwen/Qwen3.8-27B). [D3](https://huggingface.co/vllm-sr/d3) is a separate decision checkpoint from the vLLM Semantic Router community, based on Qwen3.8-27B. It answers typed questions with probabilities. It does not execute the Agent's tools or generate the task answer.

The two local models can run on the same capable machine or on separate machines you control. Allow memory for both models, execution context caches, decision inputs and the runtime. A model download size is not a RAM requirement. The full D3 is also a 27B-class model; [d3-mini](https://huggingface.co/vllm-sr/d3-mini) is a smaller alternative to evaluate, with its own accuracy and resource tradeoffs. This example is not a recommendation to run two large models inside a small Omarchy VM.

## 1. Connect local Qwen as an execution model

On your inference machine, serve the [Ollama `qwen3.8:27b` model](https://ollama.com/library/qwen3.8:27b) following the runtime's installation and model instructions. [Ollama's OpenAI compatibility documentation](https://docs.ollama.com/api/openai-compatibility) describes its local chat endpoint.

In HiRoute, open **Models → General models → Add models → New API connection**, then choose **Custom API**. For Ollama on the same machine as the HiRoute service, use:

| Field | Value |
| --- | --- |
| API base URL | `http://127.0.0.1:11434/v1` |
| Protocol | OpenAI Chat Completions |
| Model ID | `qwen3.8:27b` |
| API key | `ollama` for the local server that ignores authentication; use the configured key if a proxy requires one |

Read the model list, select the model and save. Check its declared context and tool capabilities against what your server actually supports; a successful text response alone does not verify an Agent's tool calls. Add your cloud primary model through the same page or **Connect subscription**. See [Connect models](/en/docs/models/) for the full connection flow.

`127.0.0.1` means the machine running the **HiRoute service**. If HiRoute runs in Omarchy and inference runs on the host or another LAN machine, use that server's reachable address. Keep a LAN endpoint on a private, authenticated connection. Opening a remote Web UI does not change where HiRoute sends its requests.

## 2. Connect D3 as the local decision model

Use a runtime build that supports D3 and exposes **System One**. The [vLLM Semantic Router Engine quickstart](https://github.com/vllm-project/semantic-router/blob/main/website/docs/model-runtime/get-started/quickstart.md) explains installation, release-channel requirements and model serving. Its macOS containers run on CPU; use a supported GPU deployment for this large-model example. After installing a suitable build on a capable inference machine, its Engine mode can serve the checkpoint:

```sh
vllm-sr serve vllm-sr/d3 --engine
```

The first start downloads the required image and model. Use the address printed by the server; the example below uses the documented Engine listener on port `8899`. Verify the published model ID through `/v1/systemone/models`. Engine mode supplies the decision API; HiRoute continues to own the execution route and its policies.

Alternatively, a compatible vLLM build can serve D3 directly through `/v1/systemone`; follow its [D3 support and context-limit instructions](https://docs.vllm.ai/en/latest/models/structured_decisions_models/#decision-30). Check support in the installed version before choosing it. An ordinary chat endpoint is insufficient.

In **Models → Decision models**, add a decision model, choose **Configure independently → Other compatible connection**, and enter:

| Field | Value |
| --- | --- |
| Name | `Local D3` |
| Model ID | `vllm-sr/d3`, or the exact ID published by your server |
| Complete URL | `http://127.0.0.1:8899/v1/systemone` |
| Protocol | System One |
| API Key | The endpoint's key; for a local endpoint that accepts unauthenticated requests, use a non-secret placeholder such as `local` to satisfy HiRoute's key field |

Use the complete decision URL, including `/v1/systemone`, rather than the execution model's `/v1` base URL. Match HiRoute's decision context setting to the server's actual supported input budget, allowing room for its questions. Larger context can increase memory and decision time. Do not silently truncate the server's input: losing the previous stage can invalidate its assessment.

Save and test the saved connection. The protocol match is why this design is expected to work without a custom extension: HiRoute sends `state` and named `questions`, and consumes the typed `answers`, choice probabilities and scores that System One defines. A connection test checks that exchange; it does not establish that D3 judges your tasks well.

## 3. Publish the route and select it in OpenCode

1. Open **Smart routing → New smart routing**, name the route “Local Qwen + frontier” and choose **Difficulty routing**.
2. Add local Qwen to **Regular models** and your cloud model to **Primary models**.
3. Under **Decision method**, select the saved **Local D3** connection. Keep the initial trust setting while evaluating your own tasks; review the difficulty and competence criteria under judgment settings.
4. Enable the route. Open **Agent → OpenCode → Model routing**, select it, choose a compatible protocol and save.
5. Restart OpenCode and select the route in `/models`. For screenshots of these controls, follow the [OpenCode setup guide](/en/docs/opencode-model-routing/#configure-routing); replace its illustrated models and heuristic decision method with the choices above.

On a new user turn, HiRoute can select Regular for routine work or Primary for harder work. A fresh, complete low assessment of the preceding stage can also select Primary when that earlier problem applies to the current task. A low score alone does not always cause a switch. Tool continuations reuse a valid decision for the same turn; HiRoute does not switch models midway through a streamed answer. See [Difficulty routing](/en/docs/model-routing/#difficulty-routing).

## 4. Evaluate on the deployment you choose

When inference capacity is available, use a small real task with a tool call and a follow-up. In **Sessions → Routing timeline**, inspect the actual execution model, difficulty result, previous-stage assessment and reason for each group selection. Check that a switch reflects the task and that local execution completes the required work. Do not treat an illustrative screenshot or a successful connection check as an end-to-end demonstration.

Review failure behavior too. If the whole difficulty decision fails or times out, HiRoute falls back to heuristic rules; it does not guarantee a cloud selection. A usable decision whose degree judgment is unavailable selects Primary. Execution candidates must satisfy the request's capabilities, and unavailable Regular candidates can relay to Primary. Keep cloud credentials and limits usable when you rely on that backup.

Local decisions keep the **decision inference** on your infrastructure. A turn routed to the cloud still sends that provider the context needed to execute it. To require all execution to remain local, use only local execution candidates; this hybrid route does not enforce that policy.

Set actual API prices for the cloud candidates under **Edit pricing**. Zero API-token charges for a local server do not mean zero hardware or electricity cost. Compare task completion, decision overhead and cloud usage together before increasing the local model's share. The [routing review guide](/en/docs/optimize-model-routing/) explains how to use those records.
