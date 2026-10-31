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
enforces this condition on publication and reopen; the Agent catalog uses the route
display name when its saved purpose is empty, preserving its nonempty description.
