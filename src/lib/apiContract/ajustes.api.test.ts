// Contract: the preference, security, data-location and database-backup commands are called
// from api.ts with the argument names their Rust signatures declare (see ./contract.ts), and
// their payloads match the hand-written TypeScript interfaces.
import { describe, expect, it } from 'vitest';
import {
  itMatchesRustArguments,
  rustEnumVariants,
  rustStructFields,
  tsInterfaceFields,
  tsStringUnion,
} from './contract';

describe('api.ts ↔ Rust: preference and security commands', () => {
  itMatchesRustArguments(['preferences.rs', 'security.rs', 'data_location.rs', 'backup.rs']);

  it.each(['DataLocationInfo', 'PickedDataFolder', 'DataMoveProgress'])('%s has the fields Rust sends', (name) => {
    expect(rustStructFields('commands/data_location.rs', name)).toEqual(tsInterfaceFields(name, 'src/lib/api.ts'));
  });

  it.each(['BackupStatus', 'BackupCopy'])('%s has the fields Rust sends', (name) => {
    expect(rustStructFields('services/backup/mod.rs', name)).toEqual(tsInterfaceFields(name, 'src/lib/api.ts'));
  });

  it('BackupFrequency has the values Rust accepts', () => {
    expect(tsStringUnion('BackupFrequency', 'src/lib/api.ts')).toEqual(
      rustEnumVariants('services/backup/mod.rs', 'BackupFrequency'),
    );
  });
});
