// Guards against the v0.5.0 regression where the email body iframe collapsed to
// ~40px in production. The iframe injects BRIDGE_SCRIPT as an inline <script>
// into its srcdoc; a srcdoc frame inherits the parent document's CSP, so an
// inline script is only allowed if its sha256 hash is present in `script-src`.
// Dev relaxes the CSP (Vite HMR), which is why the bug only showed in release
// builds. If BRIDGE_SCRIPT changes, its hash changes too — this test forces the
// production CSP to be updated in lockstep.
//
// Only the base config defines app.security.csp. tauri.intel.conf.json overrides
// only bundle.resources and is merged on top of the base, so it inherits this CSP.

import { describe, expect, it } from 'vitest';
import tauriConf from '../../../src-tauri/tauri.conf.json';
import { BRIDGE_SCRIPT } from './EmailHtmlFrame';

async function expectedScriptHash(): Promise<string> {
  const bytes = new TextEncoder().encode(BRIDGE_SCRIPT);
  const digest = await crypto.subtle.digest('SHA-256', bytes);
  const b64 = btoa(String.fromCharCode(...new Uint8Array(digest)));
  return `'sha256-${b64}'`;
}

function scriptSrcDirective(csp: string): string {
  const directive = csp
    .split(';')
    .map((d) => d.trim())
    .find((d) => d.startsWith('script-src'));
  if (!directive) throw new Error('No script-src directive in CSP');
  return directive;
}

function directive(csp: string, name: string): string {
  const found = csp
    .split(';')
    .map((d) => d.trim())
    .find((d) => d.startsWith(`${name} `) || d === name);
  if (!found) throw new Error(`No ${name} directive in CSP`);
  return found;
}

describe('EmailHtmlFrame bridge script CSP', () => {
  it('the production CSP allows the inline bridge script via its sha256 hash', async () => {
    const csp = tauriConf.app.security.csp;
    const scriptSrc = scriptSrcDirective(csp);
    expect(scriptSrc).toContain(await expectedScriptHash());
  });
});

describe('production CSP hardening', () => {
  // `blob:` was allowed in object-src/frame-src but nothing ever produced a blob
  // URL — there is no `URL.createObjectURL` call anywhere in src/. Dropping it
  // removes a plugin/frame instantiation channel at zero cost.
  it('does not allow blob: in object-src or frame-src', () => {
    const csp = tauriConf.app.security.csp;
    expect(directive(csp, 'object-src')).not.toContain('blob:');
    expect(directive(csp, 'frame-src')).not.toContain('blob:');
  });

  // `data:` however is LOAD-BEARING and must stay. AttachmentViewer builds a
  // `data:<mime>;base64,…` URI and renders it through `<object>` for PDFs and
  // `<iframe>` for HTML attachments (AttachmentTabView does the same). Removing
  // `data:` here silently breaks attachment preview — it looks like a safe
  // hardening step and is not. Pinned so nobody "tightens" it again.
  it('keeps data: in object-src and frame-src for the attachment viewer', () => {
    const csp = tauriConf.app.security.csp;
    expect(directive(csp, 'object-src')).toContain('data:');
    expect(directive(csp, 'frame-src')).toContain('data:');
  });

  it('still allows the asset protocol needed by the attachment viewer', () => {
    const csp = tauriConf.app.security.csp;
    expect(directive(csp, 'object-src')).toContain('asset:');
    expect(directive(csp, 'frame-src')).toContain('asset:');
  });

  it('keeps script-src free of unsafe-inline and unsafe-eval', () => {
    const scriptSrc = scriptSrcDirective(tauriConf.app.security.csp);
    expect(scriptSrc).not.toContain('unsafe-inline');
    expect(scriptSrc).not.toContain('unsafe-eval');
  });
});
