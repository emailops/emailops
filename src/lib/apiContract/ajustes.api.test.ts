// Contract: the preference, security and data-location commands are called from api.ts with
// the argument names their Rust signatures declare (see ./contract.ts), and the data-location
// payloads match their hand-written TypeScript interfaces.
import { describe, expect, it } from 'vitest';
import { itMatchesRustArguments, rustStructFields, tsInterfaceFields } from './contract';

describe('api.ts ↔ Rust: preference and security commands', () => {
  itMatchesRustArguments(['preferences.rs', 'security.rs', 'data_location.rs']);

  it.each(['DataLocationInfo', 'PickedDataFolder', 'DataMoveProgress'])('%s has the fields Rust sends', (name) => {
    expect(rustStructFields('commands/data_location.rs', name)).toEqual(tsInterfaceFields(name, 'src/lib/api.ts'));
  });
});
