import fs from 'node:fs/promises';
import { guidePages, renderUserGuide } from '../src/lib/user-guides.mjs';

const expected = Object.keys(guidePages).flatMap(slug => ['zh', 'en'].map(language => `${slug}.${language}.md`)).sort();
const actual = (await fs.readdir(new URL('../content/guides/', import.meta.url))).filter(name => name.endsWith('.md')).sort();
if (JSON.stringify(actual) !== JSON.stringify(expected)) {
  throw new Error('Register every user guide and provide both .zh.md and .en.md sources; remove orphan guide files.');
}
for (const slug of Object.keys(guidePages)) for (const language of ['zh', 'en']) await renderUserGuide(slug, language);
console.log(`Validated ${expected.length} bilingual, Agent-first user guides.`);
