# The Mac app

`apps/macos` builds **Hive.app**: the console in its own window, with its own
Dock icon, menus and shortcuts. It is the same console the browser shows - the
pages come from the hub, unchanged - and the window paints the console's own
colours under its title bar (the sidebar's over the sidebar, the page's over
the rest, following the console's light or dark theme), so it reads as the
console with traffic lights rather than a browser around it.

```bash
apps/macos/scripts/build.sh --install   # builds Hive.app into /Applications (~/Applications when that is not writable)
open -a Hive
```

It needs Xcode or the command line tools (Swift 6) to build and macOS 13.3 or
later to run. It runs the `hive` you installed (`~/.hive/bin/hive`, or the
one the `hive.hub` service names) and never carries a copy of its own, so
self-update keeps the console current exactly as before.

What the app does on its own:

- **Finds the console.** After `hive service install` it opens the console
  the `hive.hub` service serves (the port in `~/.hive/bin/hive-hub.sh`, 8765
  by default). When launchd has the service stopped or unloaded it starts it
  (`launchctl kickstart` / `bootstrap`), and it waits out a restart - a
  self-update restarts the console several times a day - for up to 90 seconds
  before it says the console did not start, with the end of
  `~/.hive/logs/hub.log` and **Restart the Console** under it. It never starts
  a second console next to the service's. Without a service it uses a `hive
  hub` already serving on 8765, or starts one itself (`hive hub --port 8765
  --no-browser`, its output in `~/.hive/logs/hub-app.log`, run as its own
  process so macOS asks about folders on behalf of hive, as for the service).
  That console stops when you quit the app, a small watchdog stops it if the
  app ever crashes, a new hive binary restarts it, and the console's **Keep
  Hive running** hands it over to the service as it does from a terminal. A
  machine with no store yet shows **Add your first project**: the folder you
  choose is registered as `hive init` registers it, and the console starts.
- **Rides out restarts.** A page that is up keeps showing while the console
  restarts (its own *Reconnecting* dot says so, and it reloads itself after an
  update). A page that cannot load shows *Reconnecting to Hive…* and comes back
  by itself, on the same page, the moment the console answers; after sleep the
  app checks again.
- **Keeps links where they belong.** Console pages open in the window, or in a
  new tab for ⌘-click and the console's own new-tab links. Any other web link
  you click - a pull request, docs, the **Open** of a Page tab, the page an
  agent drove on a dev server - opens in the **browser panel** beside the
  console, the way the Copilot app opens a page beside its chat: a small
  browser of its own with back, forward, reload, an address you can type in
  (`localhost:3000` and `127.0.0.1:…` get `http://`, the rest `https://`),
  **Open in Browser** to hand the page to your default browser, and ✕ to put
  it away; ⇧⌘B shows or hides it, dragging the divider sizes it, and a
  console link clicked in it comes back to the console. A ⌘-click, and what
  a page opens on its own (window.open, a script's redirect) - a provider's
  or MCP server's sign-in page - still goes to your default browser, which is
  where those sign-ins finish. `mailto:` links open your mail app, links to
  other apps ask first, and a file or script never replaces the console (a
  file dropped outside the chat composer is ignored). A right-click shows an
  app's menu, not a browser's: the text and image commands, and for the link
  under the pointer **Open in New Tab** (a console page) or **Open in
  Browser** (anything else, in your default browser) with **Copy Link** - no
  Open Link, Download Linked File, Back, Forward or Reload, and nothing at all
  on a bare patch of page.
- **Handles files.** The chat's paperclip opens a file panel, files dropped on
  the composer attach as in Safari, and downloads (chat artifacts, run
  transcripts) land in `~/Downloads` under a name not yet taken, with a note
  to show them in Finder.
- **Menus.** Go has ⌘1-⌘9 for Home, Chat, Tasks, Votes, Records, Agents, Runs,
  Machines and Connections (through the sidebar's own links, so the page keeps
  its data and live stream), For You, Code Delivered, ⌘[ and ⌘] for back and
  forward, and ⌘K / ⌘J for search and Ask, which the page itself answers
  first. ⌘, opens Settings. Edit has the usual text commands and find in page
  (⌘F, ⌘G, ⇧⌘G, ⌘E). View has reload (⌘R), zoom (⌘+, ⌘-, ⌘0), Appearance -
  the console's own light/dark/system switch, which the window follows - and
  **Show/Hide Browser Panel** (⇧⌘B). File
  has new windows and tabs (⌘N, ⌘T), **Open in Browser** (⇧⌘O), **Copy Link to
  This Page** (⇧⌘C) and Print (⌘P). The last page you had open comes back at
  the next launch.

Settings for unusual set-ups, all optional (`defaults write
io.github.manishtiwari25.pulse <key> <value>`):

| Key | What it does |
| --- | --- |
| `HiveHome` | another HIVE_HOME; a service that serves a different home is then not used |
| `HiveBinary` | another `hive` (the *not installed* screen offers the same choice) |
| `HubURL` | a console to show instead, never started or stopped by the app (the mock on 8787, say) |
| `HubPort` | the port for a console the app starts itself (a service's console stays on its own port) |
| `DeveloperExtras` | `-bool YES` lets Safari's Develop menu inspect the app's pages |

The app keeps its own copy of the console's browser storage (theme, view
toggles, *seen* marks), separate from your browser's. It logs to the unified
log under `io.github.manishtiwari25.pulse` (`/usr/bin/log show --predicate
'subsystem == "io.github.manishtiwari25.pulse"'`; the path, because zsh has a
`log` builtin of its own); **Help → Show Console Log** opens the console's own
log. There is no Dock badge for the **For you** count yet: that count is
worked out in the page, which stops refreshing while its window is hidden.

The console's notifications ([Settings → Notifications](console.md#notifications))
are Mac notifications in the app: the page decides what is news and hands
it to the app (the `hiveNotify` message handler, which answers only the
console's own origin in the main frame), and the app posts it through
Notification Center with the Mac's notification sound. macOS asks once,
when you turn a switch on, whether Hive may notify you; **System Settings →
Notifications → Hive** decides after that, and the card in the console says
what macOS allows. A click opens the page it is about in the window in front
and brings Hive forward. While a Hive window has focus the page shows its
own note and the app adds only the sound. When macOS does not let Hive
notify, the page plays its own chime instead. **Help → Send Test
Notification** posts one through the same path, banner and sound even while
Hive is in front, so you can see what macOS does with a real one (a click
opens Settings → Notifications); when macOS does not let Hive notify, it
says why instead. A copy run outside Hive.app
(`swift run`) has no bundle for Notification Center, so it only gets the
notes on the page. The request's parsing (and the rule that a click only
opens a console path) is `HiveKit/Notify.swift`, tested in `NotifyTests`.

`build.sh` signs ad hoc with a requirement every build meets (`designated =>
identifier "io.github.manishtiwari25.pulse"`), the idea hive's own signature
follows (ADR-035), so macOS keeps the app's answers across rebuilds, and it
links against the installed SDK rather than the deployment target. Like
hive's, that requirement names no team: any program signed ad hoc with the
same identifier meets it. A Developer ID signature ties it to you. A copy for
another Mac needs a Developer ID Application certificate and notarization:
`SIGN_IDENTITY="Developer ID Application: …" NOTARY_PROFILE=<notarytool
profile> apps/macos/scripts/build.sh --dmg` signs with the hardened runtime,
makes `Hive.dmg`, notarizes and staples it.

`apps/macos/scripts/check.sh` is the app's check: the HiveKit tests (reading
the setup, the hub's output and exits, the link policy, launchd's answers,
and the connection itself against a fake launchd, port and clock - restart
windows, a handover, a console that keeps stopping, quitting), a release
build, and a run of the built app against the mock console that goes to Tasks,
loses the console in the middle of a reload and has to come back on its own,
then clicks a web link that must open in the browser panel beside the console
(never the default browser) and puts the panel away.
