# Official Jev decision extension

[简体中文](README.zh-CN.md) · [Custom extension API](../../api/README.md)

This optional Python HTTP service implements HiRoute's current **v1** with one
System One provider call. HiRoute also has built-in connections under
**Models → Decision models**, so you only need this service when choosing a
self-hosted custom extension.

The extension follows the request's task categories, degree criteria and frozen
assessment standard. It returns a category/degree result and optional competence
for the preceding actual stage. HiRoute owns thresholds, model groups, availability
relay and observations. There are no deployment-level routing rules, thresholds
or fixed branch policies.

## Install

Use **Python 3.12 or later**. From the HiRoute repository root:

```sh
python3 -m venv "$HOME/hiroute-jev/.venv"
"$HOME/hiroute-jev/.venv/bin/pip" install ./decision-extensions/extensions/jev-decider
install -d -m 700 "$HOME/.config/hiroute"
touch "$HOME/.config/hiroute/jev-api-key"
chmod 600 "$HOME/.config/hiroute/jev-api-key"
```

Save the chosen provider's API key as UTF-8 text in
`$HOME/.config/hiroute/jev-api-key` using an editor. The examples below use that
private file; the upstream key stays on the extension host. Use an absolute path
when choosing a different file.

## Configure and start

Choose one provider configuration. The `OPENROUTER_*` variable names are used for
all compatible providers, including Bailian; there is no separate Bailian key or
account-edition variable.

### OpenRouter Jev

```sh
export OPENROUTER_API_KEY_FILE="$HOME/.config/hiroute/jev-api-key"
export OPENROUTER_DECISIONS_URL="https://openrouter.ai/api/alpha/decisions"
export JEV_MODEL="typesafe/jev-1.13"
export JEV_REQUEST_TIMEOUT_SECONDS=10
"$HOME/hiroute-jev/.venv/bin/hiroute-jev-decider"
```

### Bailian Token Plan

Use a Bailian Token Plan key in the private file, then start with:

```sh
export OPENROUTER_API_KEY_FILE="$HOME/.config/hiroute/jev-api-key"
export OPENROUTER_DECISIONS_URL="https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/systemone"
export JEV_MODEL="decision-model-preview"
export JEV_REQUEST_TIMEOUT_SECONDS=10
"$HOME/hiroute-jev/.venv/bin/hiroute-jev-decider"
```

Token Plan uses one endpoint; actual availability depends on the credential's
permissions and the provider response. For other compatible System One providers,
set the complete upstream URL and model explicitly.

### Environment variables

| Variable | Default / use |
| --- | --- |
| `OPENROUTER_API_KEY_FILE` | Required absolute path to the private UTF-8 provider key file |
| `OPENROUTER_DECISIONS_URL` | `https://openrouter.ai/api/alpha/decisions` |
| `JEV_MODEL` | `typesafe/jev-1.13` |
| `JEV_REQUEST_TIMEOUT_SECONDS` | 2.8 seconds total; includes request preparation, queueing, connection and reading; must be in `(0,3600]` |
| `JEV_MAX_REQUEST_BYTES` | 262144 bytes for the complete provider request, including questions and state; not a token estimate |
| `JEV_MAX_CONCURRENCY` | 32 concurrent upstream calls |
| `HOST`, `PORT` | `127.0.0.1`, `8080` |
| `DECIDER_AUTH_HEADER_NAME`, `DECIDER_AUTH_HEADER_VALUE` | Optional paired authentication for requests from HiRoute; configure the same header/value in the custom connection |

## Connect HiRoute to the extension

Once the service is running, `GET http://127.0.0.1:8080/health` checks the HTTP
service. It does not call the provider or verify the key's permissions.

Under **Models → Decision models**, add a **custom extension** with the complete
endpoint **`http://127.0.0.1:8080/v1/decisions`**. Choose a connection timeout longer
than the extension's total budget; the examples use a 10-second extension budget.
Set inbound authentication in both places if configured. If the service runs on a
different host, use an address reachable from HiRoute instead of loopback.
Save and test the connection, then select it in a routing plan and publish.

The two HTTP boundaries have different payloads:

| Caller → receiver | Endpoint | Payload |
| --- | --- | --- |
| HiRoute → this extension | `/v1/decisions` | The [custom extension request](../../api/README.md): definition, current input, visible history and assessment target |
| This extension → decision provider | Configured `/alpha/decisions` or `/systemone` URL | System One `model`, `state` and typed Choice/Score questions |

Do not use the provider URL as this extension's endpoint in HiRoute. To call a
provider directly, add a built-in decision model instead.

## Behavior and limits

The extension submits independent category, degree and optional assessment questions
in one provider request. Only the selected category's degree is consumed; an invalid
unselected answer does not invalidate that path. Degree uses the complete probability
distribution. Competence uses the raw provider score divided by two, not confidence.
An invalid selected degree leaves the category selected so HiRoute can use its primary
group; invalid assessment is omitted. The extension does not invent assessment reasons.

Current input and supplied instructions are preserved. Oldest whole history turns
may be removed to fit the complete-request byte budget. Loss within the assessment
target marks the score partial; if the whole target is removed, assessment is omitted.
If current input and questions alone exceed the budget, the service rejects the
request instead of truncating them. Provider context limits still apply, and the
business models keep the context settings from the routing plan.

There is one upstream call with no retry. Redirects are disabled, upstream responses
are bounded to **64 KiB**, and the service budget includes waiting for a concurrency
slot. Diagnostics record timing, counts, status and reported usage without prompts
or credentials. Tool subset selection is future documentation only and is not
implemented by this service.

## Offline verification

From the repository root, after installation:

```sh
cd decision-extensions/extensions/jev-decider
"$HOME/hiroute-jev/.venv/bin/python" -m unittest discover -v
```

The tests cover strict requests, typed reduction, trimming, authentication, deadlines
and the HTTP handler with local fixtures. They do not establish provider decision
quality or native Desktop acceptance. HiRoute's explicit connection test validates
the saved connection and required result fields; task-specific quality still needs
representative real usage.
