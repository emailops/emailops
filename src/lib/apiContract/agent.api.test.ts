// Contract: the email agent commands are called from api.ts with the argument
// names their Rust signatures declare, and answer the shapes the TS types declare.
import { describe, expect, it } from 'vitest';
import {
  itMatchesRustArguments,
  rustEnumVariants,
  rustStructFields,
  tsInterfaceFields,
  tsStringUnion,
} from './contract';

describe('api.ts ↔ Rust: email agent commands', () => {
  itMatchesRustArguments(['agent.rs']);
});

describe('email agent payloads have the shape the frontend types declare', () => {
  it.each([
    ['commands/agent.rs', 'AgentOverview'],
    ['models/agent.rs', 'AgentRule'],
    ['models/agent.rs', 'AgentRuleInput'],
    ['models/agent.rs', 'AgentAction'],
    ['models/agent.rs', 'AgentRun'],
    ['models/agent.rs', 'AgentPanel'],
    ['models/agent.rs', 'AgentPanelInput'],
  ])('%s %s', (file, name) => {
    expect(rustStructFields(file, name)).toEqual(tsInterfaceFields(name));
  });

  it.each([
    ['AgentTrigger', 'AgentTrigger'],
    ['AgentActionKind', 'AgentActionKind'],
    ['AgentActionStatus', 'AgentActionStatus'],
    ['AgentRunStatus', 'AgentRunStatus'],
    ['PanelWindow', 'PanelWindow'],
  ])('enum %s', (rust, ts) => {
    expect(rustEnumVariants('models/agent.rs', rust)).toEqual(tsStringUnion(ts));
  });
});
