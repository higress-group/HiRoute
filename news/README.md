# HiRoute Articles

| Date | English | 简体中文 |
| --- | --- | --- |
| 2026-11-27 | [Bailian releases its official decision model, and HiRoute beats OpenAI Decisions in testing](2026-11-27-bailian-stable-decision.en.md) | [百炼推出正式版决策模型，实测超越 OpenAI Decisions](2026-11-27-bailian-stable-decision.zh-CN.md) |
| 2026-11-19 | [What 30 billion tokens taught me about AI coding](2026-11-19-ai-coding-token-efficiency.en.md) | [300 亿 Token 的 AI Coding 实践](2026-11-19-ai-coding-token-efficiency.zh-CN.md) |
| 2026-11-18 | [Routine work stays local, hard turns go to the cloud: a d3 test run](2026-11-18-local-d3.en.md) | [日常留在本地，难了再上云：本地 d3 决策实测](2026-11-18-local-d3.zh-CN.md) |
| 2026-11-03 | [New HiRoute release](2026-11-03-new-model-trials-and-task-routing.en.md) | [HiRoute 新版本发布](2026-11-03-new-model-trials-and-task-routing.zh-CN.md) |
| 2026-11-03 | [A Super Agent with Hermes and HiRoute](2026-11-03-hermes-super-agent.en.md) | [用 Hermes 和 HiRoute 搭 Super Agent](2026-11-03-hermes-super-agent.zh-CN.md) |
| 2026-10-14 | [Switching models loses cache, not the savings](2026-10-14-routing-cache-cost.en.md) | [切换模型会损失缓存，智能路由还省钱吗？](2026-10-14-routing-cache-cost.zh-CN.md) |
| 2026-10-04 | [Smart routing: Astra × Qwen](2026-10-04-astra-qwen.en.md) | [智能路由省钱实战](2026-10-04-astra-qwen.zh-CN.md) |
| — | [HiRoute is open source](hiroute-open-source.en.md) | [HiRoute 正式开源](hiroute-open-source.zh-CN.md) |

Articles here are the canonical source for the bilingual website Articles pages. Add a dated Markdown pair and an entry in `index.json` for future posts. A localized `shortTitle` is optional for the compact homepage link.

Omit `date` for a backfilled introduction whose original publication date is not recorded. It follows dated news in the article list and does not replace the latest dated article on the homepage. The open-source introduction is such a backfill.

The cache-cost article uses frozen experiment prices and usage. Its conditional cache model and post-hoc cost sensitivity analysis are distinct from measured cache-hit rates. Recalculate its figures with `python3 experiments/cache_cost_analysis.py` from the repository root after fetching the public evidence archive; keep historical evidence unchanged.
