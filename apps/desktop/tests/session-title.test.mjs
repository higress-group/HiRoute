import assert from 'node:assert/strict';
import test from 'node:test';

import { firstRealUserText, isInjectedEnvironmentContext, splitLeadingSystemReminders } from '../src/features/session-title.ts';

test('only a complete standalone injected environment block is skipped', () => {
  assert.equal(isInjectedEnvironmentContext('  <environment_context>\nrepo facts\n</environment_context>\n'), true);
  assert.equal(isInjectedEnvironmentContext('<environment_context>unfinished'), false);
  assert.equal(isInjectedEnvironmentContext('Please explain <environment_context> in docs'), false);
  assert.equal(isInjectedEnvironmentContext('<environment_context>x</environment_context> keep this'), false);
  assert.equal(isInjectedEnvironmentContext('<environment_context>x</environment_context><environment_context>y</environment_context>'), false);
});

test('title selection skips injected setup and later adopts the first real user input', () => {
  assert.equal(firstRealUserText(['', ' <environment_context>setup</environment_context> ', '\n真实问题\n']), '\n真实问题\n');
  assert.equal(firstRealUserText(['<environment_context>setup</environment_context>']), null);
  assert.equal(firstRealUserText(['How do I write <environment_context>?']), 'How do I write <environment_context>?');
});

test('Claude reminder prelude does not replace the user question as the title', () => {
  assert.equal(firstRealUserText(['<system-reminder>currentDate: today</system-reminder>\n真实问题']), '\n真实问题');
  assert.equal(firstRealUserText(['<system-reminder>one</system-reminder>\n<system-reminder>two</system-reminder>\n问题']), '\n问题');
  assert.equal(firstRealUserText(['<system-reminder>context only</system-reminder>']), null);
  assert.equal(firstRealUserText(['<system-reminder>unfinished\n问题']), '<system-reminder>unfinished\n问题');
  assert.equal(firstRealUserText(['How does <system-reminder> work?']), 'How does <system-reminder> work?');
  assert.equal(firstRealUserText(['<environment_context>setup</environment_context> keep this']), '<environment_context>setup</environment_context> keep this');
});

test('complete Claude reminder prelude is separately displayable without losing captured text', () => {
  const captured = ' <system-reminder>date</system-reminder>\n<system-reminder>rules</system-reminder>\nQuestion';
  const split = splitLeadingSystemReminders(captured);
  assert.equal(split.body, '\nQuestion');
  assert.equal(split.context + split.body, captured);
  assert.deepEqual(splitLeadingSystemReminders('Question <system-reminder>literal</system-reminder>'), { context: '', body: 'Question <system-reminder>literal</system-reminder>' });
  assert.deepEqual(splitLeadingSystemReminders('<system-reminder>unfinished'), { context: '', body: '<system-reminder>unfinished' });
});

test('a complete Codex context envelope is separate from the first real question', () => {
  const prelude = '<recommended_plugins>plugins</recommended_plugins>\n# AGENTS.md instructions for /tmp\n\n<INSTRUCTIONS>rules</INSTRUCTIONS>\n<environment_context>cwd</environment_context>';
  assert.equal(firstRealUserText([prelude, 'HIR-TOOL-OK-SEP21']), 'HIR-TOOL-OK-SEP21');
  assert.equal(firstRealUserText(['# AGENTS.md instructions\n\n<INSTRUCTIONS>rules</INSTRUCTIONS>', 'Real request']), 'Real request');
  assert.equal(firstRealUserText(['# AGENTS.md instructions\n\n<INSTRUCTIONS>rules</INSTRUCTIONS>\nReal request']), '\nReal request');
  assert.equal(firstRealUserText(['# AGENTS.md instructions\n<INSTRUCTIONS>unfinished', 'Real request']), '# AGENTS.md instructions\n<INSTRUCTIONS>unfinished');
  assert.equal(firstRealUserText([prelude + '\nActual question']), '\nActual question');
  assert.equal(firstRealUserText(['Please explain <recommended_plugins>']), 'Please explain <recommended_plugins>');
  assert.equal(firstRealUserText(['<recommended_plugins>unfinished', 'Actual question']), '<recommended_plugins>unfinished');
});
