#!/usr/bin/env node
// Thin WebDriver client for the verify-emailops skill. Talks to the embedded
// W3C server the app starts when launched with `--features webdriver` and
// `TAURI_WEBDRIVER_PORT` (see src-tauri/src/webdriver.rs). One session per call.
//
//   wd.mjs status
//   wd.mjs find   <selector>              # count + text of each match
//   wd.mjs text   <selector>              # text of the first match
//   wd.mjs exists <selector>              # exit 0 if present, 1 if not
//   wd.mjs click  <selector>
//   wd.mjs type   <selector> <text>       # click, clear, type
//   wd.mjs keys   <key>                   # Enter, Escape, Tab, …
//   wd.mjs js     <expression>            # evaluated in the page, result printed as JSON
//   wd.mjs shot   <file.png>              # page screenshot
//
// Selectors are WebdriverIO selectors: CSS, `button=Inbox` (exact text),
// `button*=Nadia` (partial text), `aria/Close chat panel` (accessible name).
// Env: TAURI_WEBDRIVER_PORT (default 4445).
import { remote } from 'webdriverio';

const port = Number(process.env.TAURI_WEBDRIVER_PORT || 4445);
const [cmd, a1, a2] = process.argv.slice(2);

function usage(code) {
  console.error('usage: wd.mjs status|find|text|exists|click|type|keys|js|shot …');
  process.exit(code);
}
if (!cmd) usage(2);

if (cmd === 'status') {
  const r = await fetch(`http://127.0.0.1:${port}/status`);
  console.log(JSON.stringify(await r.json()));
  process.exit(r.ok ? 0 : 1);
}

const browser = await remote({
  hostname: '127.0.0.1',
  port,
  path: '/',
  capabilities: {},
  logLevel: 'error',
});

let exit = 0;
try {
  switch (cmd) {
    case 'find': {
      const els = await browser.$$(a1);
      console.log(`matches=${els.length}`);
      for (const el of els.slice(0, 20)) console.log('  ' + (await el.getText()).replace(/\s+/g, ' ').slice(0, 120));
      exit = els.length ? 0 : 1;
      break;
    }
    case 'text': {
      const el = await browser.$(a1);
      console.log(await el.getText());
      break;
    }
    case 'exists': {
      exit = (await browser.$(a1).isExisting()) ? 0 : 1;
      console.log(exit === 0 ? 'present' : 'absent');
      break;
    }
    case 'click': {
      const el = await browser.$(a1);
      await el.waitForClickable({ timeout: 5000 });
      await el.click();
      console.log('clicked');
      break;
    }
    case 'type': {
      const el = await browser.$(a1);
      await el.click();
      await el.setValue(a2 ?? '');
      console.log('typed');
      break;
    }
    case 'keys': {
      await browser.keys(a1);
      // The embedded server dispatches synthetic DOM key events, which never
      // trigger a form's implicit submission the way a real Enter does. Do
      // that part ourselves so a search box or login form behaves as for a user.
      if (a1 === 'Enter') {
        await browser.execute(() => {
          const el = document.activeElement;
          const form = el && el.closest ? el.closest('form') : null;
          if (form && el.tagName !== 'TEXTAREA') form.requestSubmit();
        });
      }
      console.log('sent');
      break;
    }
    case 'js': {
      const result = await browser.execute(new Function(`return (${a1});`));
      console.log(JSON.stringify(result));
      break;
    }
    case 'shot': {
      await browser.saveScreenshot(a1);
      console.log(a1);
      break;
    }
    default:
      usage(2);
  }
} catch (err) {
  console.error(`wd ${cmd}: ${err.message}`);
  exit = 1;
} finally {
  await browser.deleteSession().catch(() => {});
}
process.exit(exit);
