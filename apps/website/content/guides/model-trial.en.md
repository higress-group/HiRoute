# Try a new model

When a new model is released, you do not have to hand it all of your work at once. A HiRoute trial lets the new model take the simple turns first while a familiar primary model backs it up. If the new model falls short on a step and the problem still matters for the work ahead, the primary model takes over. Then you adjust how much work the new model takes, based on records from your own tasks.

## Start from Home

**New models** on Home lists recommendations maintained by hiroute.ai, and marks whether each one is ready to try:

- **Connected and ready to try:** select **Try it**.
- **Needs a connection:** select **Connect and try** and connect it inside the trial flow.
- If the model you want is not featured, select **Try another model** and pick one of your connected models.

The trial has four steps: choose models, review and publish, map the Agent, and start using it.

<figure>
  <a href="/guide-assets/trial-entry-en.webp"><img src="/guide-assets/trial-entry-en.webp" alt="New-model trial card on Home" width="1200" height="900" loading="lazy" /></a>
  <figcaption>The trial entry brings new models into one flow. Reuse a connection or add one as needed. Recommendations change; the pictured models are examples.</figcaption>
</figure>

## 1. Choose models

- **Model to try:** choose a connected model, or reuse a local subscription, choose a saved API model, or connect another source right here.
- **Primary model:** takes complex tasks and turns that need protection. It is prefilled only when saved settings identify a single model.
- **Decision connection:** a trial needs a competence score for every stage, so it requires a decision model; heuristic rules are not offered. If you have no decision model yet, add one here and your draft is retained.
- **Agent:** choose the Agent that will use this route.
- **Name and starting trust:** starting trust defaults to **Trial**, so the new model initially takes only the simplest work. You can adjust it at any time.

<figure>
  <a href="/guide-assets/trial-setup-en.webp"><img src="/guide-assets/trial-setup-en.webp" alt="First trial step: choose the model to try" width="1200" height="900" loading="lazy" /></a>
  <figcaption>Choose the trial model, then continue down the form to choose a primary model, decision connection and Agent. Use Review changes in the footer for the next step.</figcaption>
</figure>

## 2. Review and publish

Every change is listed before publishing. While reviewing, you can edit the actual prompt used by the competence criteria. The route starts serving new requests only after you select **Save and publish route**.

## 3. Map the Agent

Map the published route to your Agent. This changes only the selected model routing; permissions, tool approval and task collaboration stay unchanged.

- **Codex:** **On demand · CLI only** is recommended. The route becomes the default model of the HiRoute profile, while ordinary Codex and Desktop keep their settings. If you already use **Use by default**, restart the corresponding Codex CLI or Desktop after mapping.
- **Claude Code:** choose which native presets to map: Opus, Sonnet or Haiku.
- For other Agents, select the route in the Agent's model choices as the page describes.

If you leave the trial flow, you can resume mapping from **Smart routing**.

## 4. Start using it

Start a new session as the page describes. For Codex on demand, copy the launch command and run it in a terminal on the computer where Codex is installed; running ordinary Codex or Desktop does not use the trial configuration.

Begin with a well-scoped task, then watch the routing timeline in **Sessions**: which model ran each turn and why, how each stage scored, and where low-score protection applied and was released. Costs are API-equivalent estimates.

## Adjust trust with evidence

Trust is a difficulty bar: further right sends more simple work to the new model. Besides the five presets, **Trial, Familiar, Trusted, Hands-off** and **Entrusted**, you can enter an exact value. Applicable low-score protection still applies even at **Entrusted**.

When you adjust **Trust** in the route editor, **Replay this setting on your records** uses the active version's history to estimate how many stages would go to the regular model at the new level. It estimates past allocation; it is not a prediction.

<details>
<summary>When do trust suggestions appear?</summary>

With enough evidence, **Needs your attention** on Home shows a suggestion. Suggestions use only records of the active published version:

- **Step up:** at least 30 stages with complete scores, from at least 3 sessions, with at least 60% scoring coverage, and at least 85% of them meeting the competence floor. If the model has an update announcement in that period, only records after the announcement count.
- **Step down:** among at least 30 completely scored stages, fewer than 70% meet the competence floor. At the most cautious level, the suggestion becomes returning to the previous routing or changing the regular model.
- **Divide work:** when trust is already at **Hands-off** or further right and the model still performs steadily, you can divide work by task category and give one kind of task to the regular model.

</details>

Suggestions are only suggestions. Opening one creates a separate draft; you still confirm saving and publication, and Agent mappings do not change automatically. When there is no suggestion, keeping the current level is a normal outcome.

The route's **Model performance** summarizes each model's stages, competence rate, scoring coverage and cost estimate, so you can read it alongside a suggestion. Providers may update a model behind the same name, so records may mix versions.

<figure>
  <a href="/decision-assets/readme-report-en.png"><img src="/decision-assets/readme-report-en.png" alt="Route model-performance report with scoring coverage and cost estimates" loading="lazy" /></a>
  <figcaption>Check quality and cost under Smart routing → Model performance. The values illustrate the interface; they are not a benchmark or a savings promise.</figcaption>
</figure>

**How do you know it works?** The route is published, the Agent has loaded it, and a real execution appears in Sessions. Accumulate evidence across tasks before deciding whether to increase trust.
