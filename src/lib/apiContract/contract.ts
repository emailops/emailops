// Static contract between `src/lib/api.ts` and the Rust command layer, read
// from the sources: no app, no IPC. Tauri matches command arguments by name
// (camelCase on the JS side for a snake_case Rust parameter) and drops what it
// does not know, so a renamed parameter fails only at runtime — or silently,
// when the parameter is an `Option`. These helpers make that a test failure.
import { expect, it } from 'vitest';
import apiSource from '@/lib/api.ts?raw';
import typesSource from '@/types/index.ts?raw';

/** Rust sources by path relative to `src-tauri/src/`: every command file, and the files whose types cross the boundary. */
const RUST_SOURCES: Record<string, string> = Object.fromEntries(
  Object.entries(
    import.meta.glob<string>(
      [
        '../../../src-tauri/src/commands/*.rs',
        '../../../src-tauri/src/models/mod.rs',
        '../../../src-tauri/src/ai/provider.rs',
        '../../../src-tauri/src/services/ai_activity.rs',
        '../../../src-tauri/src/services/calendar/invite.rs',
      ],
      { query: '?raw', import: 'default', eager: true },
    ),
  ).map(([path, source]) => [path.replace(/^.*\/src-tauri\/src\//, ''), source]),
);
const TS_SOURCES: Record<string, string> = { 'src/types/index.ts': typesSource, 'src/lib/api.ts': apiSource };

function rustSource(relPath: string): string {
  const source = RUST_SOURCES[relPath];
  if (source === undefined) throw new Error(`${relPath} is not among the Rust sources this contract reads`);
  return source;
}

export interface RustCommand {
  name: string;
  file: string;
  /** Argument names as the frontend must spell them. */
  params: string[];
  /** The ones that are not `Option<…>`. */
  required: string[];
}

export interface InvokeSite {
  command: string;
  /** Top-level keys of the argument object; null when it is not a literal. */
  keys: string[] | null;
}

const camel = (s: string) => s.replace(/_([a-z0-9])/g, (_, c: string) => c.toUpperCase());

/** Index of the bracket that closes the one at `open`. */
function closing(text: string, open: number): number {
  const pairs: Record<string, string> = { '(': ')', '{': '}', '[': ']', '<': '>' };
  const close = pairs[text[open]];
  let depth = 0;
  for (let i = open; i < text.length; i++) {
    // `->` and `=>` are arrows, not brackets.
    if (text[i] === '>' && (text[i - 1] === '-' || text[i - 1] === '=')) continue;
    if (text[i] === text[open]) depth++;
    else if (text[i] === close && --depth === 0) return i;
  }
  throw new Error(`unbalanced ${text[open]} at ${open}`);
}

/** Split at commas that are not inside brackets. */
function splitTopLevel(text: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if ('({[<'.includes(c)) depth++;
    else if (')}]'.includes(c) || (c === '>' && text[i - 1] !== '-' && text[i - 1] !== '=')) depth--;
    else if (c === ',' && depth === 0) {
      parts.push(text.slice(start, i));
      start = i + 1;
    }
  }
  parts.push(text.slice(start));
  return parts.map((p) => p.trim()).filter(Boolean);
}

// Line comments: at the start of a line, or after whitespace (so the `//` of a URL in a string stays).
const stripComments = (src: string) => src.replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|\s)\/\/.*$/gm, '$1');

/** Every `#[tauri::command]` of one file under `src-tauri/src/commands/`. */
export function rustCommands(file: string): RustCommand[] {
  const src = stripComments(rustSource(`commands/${file}`));
  const commands: RustCommand[] = [];
  const re = /#\[tauri::command([^\]]*)\]\s*(?:#\[[^\]]*\]\s*)*pub\s+(?:async\s+)?fn\s+([a-z0-9_]+)\s*(?:<[^(]*>)?\(/g;
  for (let m = re.exec(src); m; m = re.exec(src)) {
    const open = m.index + m[0].length - 1;
    const snakeCase = /rename_all\s*=\s*"snake_case"/.test(m[1]);
    const args = splitTopLevel(src.slice(open + 1, closing(src, open)))
      .map((p) => {
        const colon = p.indexOf(':');
        return {
          name: p
            .slice(0, colon)
            .replace(/^mut\s+/, '')
            .trim(),
          type: p.slice(colon + 1).trim(),
        };
      })
      // What Tauri injects rather than reads from the call.
      .filter((p) => !/^(tauri::)?(State|AppHandle|Window|WebviewWindow|ipc::Channel|Channel)\b/.test(p.type));
    const spell = (n: string) => (snakeCase ? n : camel(n.replace(/^_/, '')));
    commands.push({
      name: m[2],
      file,
      params: args.map((p) => spell(p.name)),
      required: args.filter((p) => !p.type.startsWith('Option<')).map((p) => spell(p.name)),
    });
  }
  return commands;
}

export const commandFiles = () =>
  Object.keys(RUST_SOURCES)
    .filter((path) => path.startsWith('commands/') && path !== 'commands/mod.rs')
    .map((path) => path.slice('commands/'.length));

/** Every `invoke('command', {…})` call in `src/lib/api.ts`. */
export function invokeSites(): InvokeSite[] {
  const src = stripComments(apiSource);
  const sites: InvokeSite[] = [];
  const re = /\binvoke\s*(?:<[^;]*?>)?\(\s*'([A-Za-z0-9_:|-]+)'\s*(,\s*)?/g;
  for (let m = re.exec(src); m; m = re.exec(src)) {
    const after = m.index + m[0].length;
    if (!m[2]) {
      sites.push({ command: m[1], keys: [] });
    } else if (src[after] === '{') {
      const body = src.slice(after + 1, closing(src, after));
      const keys = splitTopLevel(body).map((entry) => (entry.startsWith('...') ? entry : entry.split(':')[0].trim()));
      sites.push({ command: m[1], keys });
    } else {
      sites.push({ command: m[1], keys: null });
    }
  }
  return sites;
}

/**
 * One test per command of `files` that `api.ts` calls: every argument it
 * sends is one the command declares, and every argument the command cannot
 * do without is sent.
 */
export function itMatchesRustArguments(files: string[]) {
  const sites = invokeSites();
  for (const command of files.flatMap(rustCommands)) {
    const calls = sites.filter((s) => s.command === command.name);
    if (calls.length === 0) continue;
    it(`${command.name}: api.ts sends the arguments the Rust command declares`, () => {
      for (const call of calls) {
        expect(call.keys, `${command.name} is called with something other than an object literal`).not.toBeNull();
        const keys = call.keys ?? [];
        expect(
          keys.filter((k) => k.startsWith('...')),
          'a spread hides the argument names',
        ).toEqual([]);
        expect(
          keys.filter((k) => !command.params.includes(k)),
          `sent but not declared by ${command.file}`,
        ).toEqual([]);
        expect(
          command.required.filter((p) => !keys.includes(p)),
          `required by ${command.file} but not sent`,
        ).toEqual([]);
      }
    });
  }
}

/** Field names a `#[serde(rename_all = "camelCase")]` struct serialises. */
export function rustStructFields(relPath: string, name: string): string[] {
  const src = stripComments(rustSource(relPath));
  const m = new RegExp(`((?:#\\[[^\\]]*\\]\\s*)*)pub struct ${name}\\s*\\{`).exec(src);
  if (!m) throw new Error(`struct ${name} not found in ${relPath}`);
  const camelCase = /rename_all\s*=\s*"camelCase"/.test(m[1]);
  const open = m.index + m[0].length - 1;
  return splitTopLevel(src.slice(open + 1, closing(src, open)))
    .filter((f) => !/#\[serde\([^)]*skip\b/.test(f))
    .map((f) => {
      const rename = /#\[serde\([^)]*rename\s*=\s*"([^"]+)"/.exec(f);
      const field = f
        .replace(/#\[[^\]]*\]\s*/g, '')
        .replace(/^pub(\([a-z]+\))?\s+/, '')
        .split(':')[0]
        .trim();
      return rename ? rename[1] : camelCase ? camel(field) : field;
    })
    .sort();
}

/** Variant names a `#[serde(rename_all = "camelCase")]` unit enum serialises. */
export function rustEnumVariants(relPath: string, name: string): string[] {
  const src = stripComments(rustSource(relPath));
  const m = new RegExp(`pub enum ${name}\\s*\\{`).exec(src);
  if (!m) throw new Error(`enum ${name} not found in ${relPath}`);
  const open = m.index + m[0].length - 1;
  return splitTopLevel(src.slice(open + 1, closing(src, open))).map((v) => v[0].toLowerCase() + v.slice(1));
}

/** Keys of the `serde_json::json!({…})` a command function answers with. */
export function rustJsonKeys(file: string, fn: string): string[] {
  const src = stripComments(rustSource(`commands/${file}`));
  const start = src.indexOf(`fn ${fn}(`);
  if (start < 0) throw new Error(`fn ${fn} not found in ${file}`);
  const at = src.indexOf('Ok(serde_json::json!({', start);
  const open = src.indexOf('{', at);
  return splitTopLevel(src.slice(open + 1, closing(src, open)))
    .map((entry) => entry.split(':')[0].trim().replace(/"/g, ''))
    .sort();
}

/** Property names of an `export interface` in `src/types/index.ts`. */
export function tsInterfaceFields(name: string, relPath = 'src/types/index.ts'): string[] {
  const src = stripComments(TS_SOURCES[relPath] ?? '');
  const m = new RegExp(`export interface ${name}\\s*(?:extends [^{]+)?\\{`).exec(src);
  if (!m) throw new Error(`interface ${name} not found in ${relPath}`);
  const open = m.index + m[0].length - 1;
  const body = src.slice(open + 1, closing(src, open));
  const fields: string[] = [];
  let depth = 0;
  for (const line of body.split('\n')) {
    const prop = /^\s*([A-Za-z_][A-Za-z0-9_]*)\??\s*:/.exec(line);
    if (depth === 0 && prop) fields.push(prop[1]);
    for (const c of line) {
      if ('({['.includes(c)) depth++;
      else if (')}]'.includes(c)) depth--;
    }
  }
  return fields.sort();
}

/** Members of an `export type X = 'a' | 'b'` string union. */
export function tsStringUnion(name: string, relPath = 'src/types/index.ts'): string[] {
  const src = stripComments(TS_SOURCES[relPath] ?? '');
  const m = new RegExp(`export type ${name}\\s*=([^;]+);`).exec(src);
  if (!m) throw new Error(`type ${name} not found in ${relPath}`);
  return [...m[1].matchAll(/'([^']+)'/g)].map((x) => x[1]);
}
