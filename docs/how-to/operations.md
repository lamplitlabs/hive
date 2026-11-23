# Keep Hive running, updated and backed up

Run Hive as two services that start when you log in, keep it current, back up
its store, and move or remove what you no longer need.

**Before you start**

- Hive is installed and the console opens ([Install](../../README.md#install)).
- A project is registered ([Set up your first project](first-project.md)).

## 1. Run Hive as two services

Hive is two processes. The **runner** (`hive daemon`) plans jobs and starts
agents. The **console** (`hive hub`) serves the pages at
`http://127.0.0.1:8765`. Started in a terminal, each stops when that terminal
closes. As user services they start when you log in and are started again when
they exit.

In the console, open **Settings**, **This machine**. The section opens with
**Runner controls**. Press **Keep Hive running**, open **Show the files it
writes** if you want to read them first, then press **Install the services**.
If you started the console with `hive hub` in a terminal, the service's console
takes over at the same address and the page reconnects. When the section says
"Nothing to do here", Hive already runs as services.

From a terminal, stop a `hive hub` or `hive daemon` you started by hand
(Ctrl-C), then:

```bash
hive service install     # write both services and load them
hive service status      # each service file, its state, the runner's heartbeat
```

| Service | macOS (launchd) | Linux (`systemd --user`) |
| --- | --- | --- |
| Runner | `~/Library/LaunchAgents/hive.daemon.plist` | `~/.config/systemd/user/hive-daemon.service` |
| Console | `~/Library/LaunchAgents/hive.hub.plist` | `~/.config/systemd/user/hive-hub.service` |

Each service runs a launcher script in `~/.hive/bin` and writes to a log:
`~/.hive/logs/daemon.log` for the runner, `~/.hive/logs/hub.log` for the
console. `hive service install --hub-port 8766` moves the console to another
port, `--no-hub` installs the runner alone, and `--dry-run` prints the files
and writes nothing. Running the command again is safe: a service that did not
change and is loaded is left running.

## 2. Update Hive

```bash
hive update --check     # is there a newer release? Installs nothing
hive update             # download it, check it, install it
```

`hive update --check` says that the version you run is up to date, or names
the newer one and ends with `Install it with: hive update`. `hive update`
downloads the archive for your machine, checks it against the release's
`SHA256SUMS`, tries the new program and puts it in place of the old one, which
it keeps beside it as `hive.prev`. To go back, run
`mv ~/.hive/bin/hive.prev ~/.hive/bin/hive`.

If Homebrew installed your `hive`, upgrade it with `brew upgrade hivefleet`:
`hive update` leaves Homebrew's file alone and names that command.

You restart nothing: the services see the new program and restart on it. The
runner restarts as soon as no run is in flight. A run still going after
`daemon.restart_drain_minutes` (30) is asked to hold: it saves its
conversation, and the next runner carries its job on in the same clone. The
console restarts once no session is at work, or after 30 minutes.

Run by hand, both commands look at Hive's own releases on GitHub. To hear of a
release without asking, say where releases are:

```bash
hive settings set update.latest_url \
  https://api.github.com/repos/lamplitlabs/hive/releases/latest
```

The runner then looks once a day. When a release is out, `hive version` prints
a second line, and the console shows a banner above every page except
**Sessions**, for example **Hive v0.1.7 is available**, with **Update now**
and **Release notes**.

`update.channel` says whether this machine takes published releases or
rebuilds Hive from its own source
([The two channels](../release.md#the-two-channels-updatechannel)). Set
`daemon.self_update` to `release` and the runner installs each release itself
([A machine without source](../release.md#daemonself_update-release-a-machine-without-source)).

## 3. Know where your data is

Hive keeps its data in the hive home: `~/.hive`, or the folder `HIVE_HOME`
names.

| In `~/.hive` | What it holds |
| --- | --- |
| `hive.sqlite` | the store: tasks, jobs, the board, records, settings, events |
| `hive-archive.sqlite` | rows older than `daemon.archive_days` (90) that nothing reads back, moved here once a month |
| `logs/` | `daemon.log`, `hub.log` and one log for each run |
| `private/` | your model provider logins (`providers/`) and the loop's private tests (`heldout/`) |
| `work/` | the throwaway clones that jobs run in |
| `patches/` | the commits of failed and swept branches, saved as patches |
| `bin/` | the two launchers, and `hive` with `hive.prev` when the installer put it there |

The store can have two side files beside it, `hive.sqlite-wal` and
`hive.sqlite-shm`: they are part of it. Your projects are not in the hive
home: a project is the checkout you registered, and finished work is on
branches there and on its upstream.

## 4. Back up the store

Nothing regenerates the store, so keep a copy on another disk. Name a place
and the runner copies the store there every `backup.every_hours` (24), keeping
the newest `backup.keep` (14) copies in a folder or on Azure. In the console,
open **Settings**, **Limits**, **Backups**. Under **Where backups go** choose
**A folder**, **Azure Blob Storage** or **A command**, fill in what it asks
for, then press **Save backups**. In a terminal:

```bash
hive settings set backup.to /Volumes/Backup/hive   # a folder that already exists
hive backup                                        # one copy now
hive backup --status                               # the last copy, and the next
```

`backup.to` takes a folder, which must exist already (Hive never makes it, so
a drive that is not mounted is refused); `az://ACCOUNT/CONTAINER[/PREFIX]`,
uploaded with the `az` program you are logged in to; or
`run: COMMAND {file} {name}`, a command of yours with the copy's path and name
filled in.

Each copy is `hive-<host>-<stamp>.sqlite.gz`: a checked, gzipped copy of
`hive.sqlite`. `hive backup` prints where it went and its SHA-256, and a copy
that failed is tried again an hour later. A backup holds the store only: not
the archive file, not the logs or the logins, not your checkouts.

For a copy by hand, run `hive export --db ~/hive-copy.sqlite` (`--sql PATH`
writes a SQL dump instead). The path must lie outside every project.

### Restore a copy

Hive has no restore command. A backup is the store itself, gzipped
([Backups of the store](../runner.md#backups-of-the-store)), so you put the
file back. Do it only while no Hive program runs: stop both services with
`hive service uninstall`, end a `hive hub` or `hive daemon` in a terminal with
Ctrl-C, and check that no `hive` process is left. Then, with the name of your
own copy in place of the example:

```bash
mkdir ~/hive-before-restore
mv ~/.hive/hive.sqlite* ~/hive-before-restore/     # the store and its side files
gunzip -c /Volumes/Backup/hive/hive-my-mac-20261123T041500Z.sqlite.gz \
  > ~/.hive/hive.sqlite
hive status                                        # the projects and jobs of the copy
hive service install                               # start the services again
```

The store is now what it was when the copy was made: tasks, posts and events
written after it are gone. Your checkouts and their branches are not touched.
A copy made with `hive export --db` goes back the same way, with `cp` in place
of `gunzip`.

## 5. Use another disk, and keep the machine awake

Clones, build caches and worktrees are most of what Hive writes, and all of it
can be made again. `hive bulk attach /Volumes/NAME` moves them to a folder
`hive` on that volume and leaves the store where it is, and `hive bulk detach`
brings them back. While the disk is away the runner holds all work
([Bulk storage on another disk](../runner.md#bulk-storage-on-another-disk)).

A laptop that sleeps takes its runs with it. Set `daemon.keep_awake` to `on`
(**Settings**, **This machine**, **Staying awake**) and the runner holds the
machine awake while it is plugged in; `always` holds it on battery too. A
closed lid still sleeps a Mac that has no external display
([Keeping the machine awake](../runner.md#keeping-the-machine-awake)).

## 6. Move a checkout, or remove a project

When a project's folder moves, point the project at the new place. In the
console, open the project, then **More actions**, **Change folder…**. In a
terminal, run `hive move my-app ~/work/my-app`. The slug, the tasks and the
history stay, and nothing on disk changes. A project whose folder is gone
shows **Missing on disk** and the button **Point to its folder**.

Removing a project deactivates it: its queued jobs are cancelled and nothing
is deleted. In the console it is **More actions**, **Remove project…**. In a
terminal:

```bash
hive remove project my-app --dry-run    # the preview only
hive remove project my-app              # the preview, then "Apply? [y/N]"
```

```text
Changes
  project my-app                 status active -> inactive
  3 queued jobs                  -> cancelled
```

The posts, runs, records, events and the checkout stay. To bring the project
back, press **Restore** on its **Removed** card in the project list, or run
`hive init ~/code/my-app`.

`hive remove project my-app --forget` is the hard delete, for a project that
is already inactive and whose checkout is gone from disk: it deletes the
project's posts, jobs, runs, records and events.

## 7. Remove Hive

1. Take a last copy of the store if you may want it (`hive export`, step 4).
2. Run `hive service uninstall`, so that nothing starts again. It unloads both
   services and deletes their files and launchers. Stop a `hive hub` or
   `hive daemon` you started by hand. If you ran `hive hooks install` for
   another coding agent, run `hive hooks uninstall copilot` or
   `hive hooks uninstall claude` while the program is still there.
3. On a second machine, undo what pairing or joining set up there
   ([Add a second machine](add-a-machine.md)).
4. Remove the program and the data. `rm -rf ~/.hive` takes the store, the
   archive, the logs, the logins, the work directories and the `hive` that the
   installer put in `~/.hive/bin`. A `hive` from Homebrew is not in that
   folder: remove it with Homebrew, which knows it as the formula `hivefleet`.
5. Remove what is left elsewhere, such as the backups at the place `backup.to`
   named, the folder `hive` on a disk you attached, and the `PATH` line you
   added to your shell profile.

Your checkouts are not touched: branches named `hive/...` and `evolve/...`
stay in them until you delete them.

## Check that it worked

- **Settings**, **This machine** says **Runner on**, and **Runner details**
  lists both services as loaded. In a terminal, `hive service status` shows
  both service files as `present`, and `hive status` shows `daemon     alive`.
- `hive backup --status` shows a `last:` line with the time of a copy, and
  the copy is in the place you named.
- `hive version` prints the version you installed, and **Settings**,
  **This machine** shows the same for **This console** and **Runner**.

## When something goes wrong

- **The console says a runner may already be running.** You started
  `hive daemon` in a terminal, and the console does not install over it. Stop
  it there with Ctrl-C, press **Refresh** and try again.
- **You ran `hive service install` while `hive hub` ran in a terminal.** That
  console still holds the address. Stop it: the service manager starts the
  service's console again every 30 seconds.
- **A backup failed because the folder "is not here".** The drive is not
  mounted, or the folder was never made. Mount it or make it. The runner tries
  again an hour later, and `hive backup` tries now.
- **The runner cannot find `gh`, or a tool a project needs.** The services
  keep the `PATH` of the shell that installed them. Run `hive service install`
  again from a shell that finds the tool.

## Read more

- [Troubleshooting](troubleshooting.md): symptoms and their fixes.
- [Services and self-update](../runner.md#services-and-self-update): how the
  services restart and drain.
- [Releases and `hive update`](../release.md): channels, releases, Homebrew.
- [Sweep, retention and hygiene](../recovery.md#sweep-retention-and-hygiene):
  what the runner cleans up, and how long it keeps logs and clones.
- [Removing your data](../security.md#removing-your-data): what is where.
- [Commands](../commands.md) and [Settings](../settings.md): the reference.
