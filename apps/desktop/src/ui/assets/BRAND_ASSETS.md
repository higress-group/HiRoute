# Agent brand assets

These assets identify third-party Agent integrations inside HiRoute. They do not brand HiRoute itself and do not imply sponsorship, certification, or endorsement.

## Codex

- Files: `codex-dark.png`, `codex-light.png`
- Source: the official OpenAI Codex desktop distribution installed at `/Applications/ChatGPT.app`, bundle identifier `com.openai.codex`, version `26.901.51231` (build `8109`). The original resource names are `icon-codex-dark-color.png` and `icon-codex-light.png`.
- SHA-256: dark `69fb4384e161be8a20dcb94a9ac34aea4fbfaeb67514110a71e7b0732eccb0fc`; light `de7d43f3386105ab20952958c2c25beb0d903e2aeb6e1aef57c49a648c0d1c07`.
- Conditions: keep the artwork unmodified, preserve its clear space, show it only next to the accurate Codex product name, keep HiRoute branding more prominent, and follow <https://openai.com/brand/>. OpenAI owns the mark and may require its use to change or stop.

## Claude Code

- File: `claude-code.svg`
- Source: Anthropic's official **Media assets → Download press kit**, <https://www.anthropic.com/press-kit>, archive last modified 2026-02-06. The exact archive member is `Anthropic media resources/Anthropic logos/Claude logos/4 Claude icon/SVG/ClaudeIcon-Rounded.svg`.
- SHA-256: `059e22f525d67c6258c4f64514f0b0e717c914df8a706936d0299d5e6b8082d9`.
- Conditions: the SVG is copied without shape or color changes and appears only with the accurate Claude Code product name. Anthropic owns the mark; this nominative integration label does not imply endorsement.

## Qoder

- File: `qoder.svg`
- Source: the official Qoder website icon, <https://qoder.com/favIcon.svg>, retrieved 2026-10-04.
- SHA-256: `5e1843cb50c4613855024453e76d5c06d2ca5562efebc12b4fcd80cf9ece1dd0`.
- The SVG is copied without shape or color changes and appears beside the Qoder name to identify the integration. The mark belongs to Qoder and does not imply endorsement.

## OpenCode

- Artwork: the inline `Mark` in `BrandIcon.tsx`, from the official [OpenCode v1.19.3 logo component](https://github.com/anomalyco/opencode/blob/v1.19.3/packages/ui/src/components/logo.tsx).
- The two path shapes and 16×20 proportions are preserved. Its neutral foreground and shadow follow the active HiRoute theme, with clear space inside the existing avatar.
- Use the mark beside the accurate OpenCode name to identify the integration. OpenCode owns the mark; this use does not imply endorsement.

## Hermes

- File: `hermes.svg`
- Source: the official Hermes Agent website favicon from NousResearch/hermes-agent at `ecac27c0af86064c40eeb8097cc20377cf2eee12`, <https://raw.githubusercontent.com/NousResearch/hermes-agent/ecac27c0af86064c40eeb8097cc20377cf2eee12/website/static/img/favicon.svg>.
- SHA-256: `6c4968c1a46f104559684826ed8061fad6052711ff0af9c6b36a6ce674536684`.
- The SVG is copied without artwork changes to identify Hermes. Upstream's MIT license is included in `hermes-LICENSE.txt`; the mark remains owned by its author and does not imply endorsement.

## Fallback

Unknown Agents use HiRoute's neutral inline `Agent` glyph plus their returned display name. A missing known asset is treated as an incomplete integration and must not fall back to initials such as `CX` or `CC`.
