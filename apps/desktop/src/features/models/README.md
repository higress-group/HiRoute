# Saved models and route creation

`Models` owns the saved-model list/detail view. Its primary route action appears
immediately after model identity, before defaults, pricing and source configuration.
Trials remain in their own product flow.

`CreateModelRoute` offers same-ID models from other ready saved connections and
matching existing routes. Labels are descriptive; matching uses exact upstream IDs
and stable Binding IDs. Merely opening the dialog or editor never saves a route.

`model-route.ts` builds a shared-authoring request, retaining selected candidate
order and an existing fixed route's settings/reasoning. `PlanEditor` gets current
authoring defaults and guards the exact existing head before adding candidates.
New routes prefill the upstream ID as display name and invocation name, fixed mode,
and delegation off. Native budget-only models receive an editable minimum budget
in this explicit creation flow. Save uses the ordinary protected publication path.
Unsupported aliases and concurrent changes remain explicit; existing routes are
never silently converted to another strategy.

Purpose is optional for routing, and required when enabling delegation. The Domain
enforces this condition on publication and reopen; Agent and Gateway catalogs use
the route display name when its saved purpose is empty, preserving their nonempty
description contracts.

Missing capability, exact limit and reasoning facts on saved registered OpenAI/Anthropic API models are resolved by
the shared scoped metadata default in `hiroute-integrations`. The daemon uses the
same default for model/default reads and candidate materialization. It matches the
registered target, protocol, authentication, product and exact upstream ID, and
preserves explicit source values, exact FollowOfficial intent and frozen
publications. PreserveSaved protects meaningful saved facts; an absent or generic
runtime-fallback capability fact can still receive this missing-value default.
The read projection does not rewrite stored sources or qualify entitlement.

OpenAI API-scoped provider/model records can fill absent binding metadata only
when the exact API root, authentication, protocol and ID match the installed
official connector's native effort convention. Their effort sets are ordered
natively; conflicts and withdrawn bindings stay unresolved. Subscription records
never supply API controls.

Model capability editing has two visible modes: Custom and Follow HiRoute model
metadata. Custom values, including unknown, are source-default choices sealed by
the ordinary preview/save operation; they do not rewrite source membership or
credential evidence. Legacy preserved/official-locator policies retain their exact
meaning and bytes. New model/default reads and publication share the effective
projection; already published revisions keep their frozen capabilities.
