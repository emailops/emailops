# Tag Board

A grid of blocks, one per classified tag (company, intent, topic or priority) per
account, each listing the threads carrying that tag. A range filter and a tag search
narrow the board; blocks can be hidden or reordered.

## Sub-features

- `tagboard.open` the Views entry renders blocks with tag title, thread count and rows.
- `tagboard.range` All time / Today / Yesterday / Last 7 days / Custom limit rows by date; empty blocks hide.
- `tagboard.search` "Search tags…" filters blocks by tag name.
- `tagboard.open-email` clicking a row opens the thread, same as the inbox.
- `tagboard.hide` the block ⋮ menu hides a tag; "Show N hidden tags" restores.

## How to get to it (user POV)

- Sidebar → Views → **Tag Board** (only shown when AI is enabled).
- Category selector (Primary/…) and account selection behave as in the inbox.

## Driving it with verify.sh

Preconditions: baseline; the demo DB ships classified tags, so blocks exist without running the classifier. Entry confirmed present on 11/09/2026: `button=Tag Board`.

- Open → `$V wd click 'button=Tag Board'`, `$V wd shot "$R/tagboard-open.png"` → `$V wd exists 'input[placeholder^="Search tags"]'` prints `present` and `$V wd find 'button=All time'` one match.
- Range → `$V wd click 'button=Today'` → fewer `div[role="button"]` rows than before (the demo mail is not dated today); `$V wd click 'button=All time'` restores them.
- Search → `$V wd type 'input[placeholder^="Search tags"]' 'codeberg'` → only blocks whose title contains *codeberg* remain; `$V wd exists '*=No tags match'` prints `absent`.
- Open email → `$V wd click '//div[@role="button"][contains(., "Codeberg")]'` → `$V wd exists 'button=Back'` prints `present`.
- Hide → `$V wd click 'aria/Block menu'` then the Hide item → `$V wd find '*=hidden tag'` shows the "Show 1 hidden tags" control; the hidden state is a `user_preferences` row (`sqlite3 … "select key from user_preferences where key like '%tagboard%'"`).

## Gotchas

- Block titles are `<account> · <tag>`; match on the tag fragment.
- Density (granular/extended) and drag-reorder are pointer gestures; use cua-driver `drag` for those, not WebDriver.
- Custom range renders two date inputs; type dates as `YYYY-MM-DD` per field, then Tab.
