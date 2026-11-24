# Run Hive on Windows

Install Hive inside WSL, the Linux that Windows runs, open the console in
your Windows browser, and keep Hive running after you close the window.

Hive does not run on Windows itself yet. Inside WSL it is on a Linux like
any other, so everything in these guides and in the reference applies
there as written.

**Before you start**

- Windows 11, or Windows 10 version 2004 or newer. That is what Microsoft
  asks for `wsl --install`.
- A PC with an x86_64 processor. The steps were tried there, on Windows
  Server 2025 with WSL 2. On an arm64 PC the installer takes Hive's arm64
  Linux release, which runs on other arm64 Linux machines but has not been
  tried under WSL.
- A model: a GitHub account with a Copilot seat, an API key for another
  provider, or a local model server.

## 1. Know what runs where

Hive runs in the Linux: the program, its store (`~/.hive/hive.sqlite` in
your Linux home), the runner, and every command a job runs. Windows gives
you two things, the window you type in and the browser that shows the
console.

So what a job needs has to be in the Linux, not in Windows: `git`, the
GitHub CLI (`gh`), and your project's own compilers and package managers.
A program you installed in Windows does not count, even when the Linux can
see it.

Keep your projects in the Linux too, under your Linux home
(`~/code/my-app`), and not under `/mnt/c`, which is your Windows disk seen
from the Linux. Microsoft's advice is to keep files on the side whose tools
work on them, because crossing over is slow
([Working across file systems](https://learn.microsoft.com/windows/wsl/filesystems)).
Hive's clones and its store are in the Linux home already.

## 2. Install WSL

Open PowerShell as administrator (right-click it, **Run as administrator**)
and run:

```powershell
wsl --install
```

Restart Windows when it asks. After the restart a window opens for Ubuntu,
the Linux that Windows installs unless you name another, and asks for a
user name and a password. They are the Linux's own, and have nothing to do
with your Windows account.
[Install WSL](https://learn.microsoft.com/windows/wsl/install) is
Microsoft's page for this step, with what to do when WSL is already there
or the command prints only its help.

Then check two things. In PowerShell:

```powershell
wsl --list --verbose      # your Linux, with 2 under VERSION
```

and in the Linux:

```bash
ps -p 1 -o comm=          # prints: systemd
```

`systemd` is what step 5 needs. The Ubuntu that `wsl --install` gives has
it. Where the line prints something else, turn it on as Microsoft's
[systemd page](https://learn.microsoft.com/windows/wsl/systemd) says: add
`[boot]` and `systemd=true` to `/etc/wsl.conf`, run `wsl --shutdown` in
PowerShell, and open the Linux again.

## 3. Install Hive in the Linux

Open the Linux: **Ubuntu** in the Start menu, or `wsl` in PowerShell. Then
install Hive as on any Linux:

```bash
curl -fsSL https://hivefleet.dev/install.sh | sh
export PATH="$HOME/.hive/bin:$PATH"    # add to your shell profile
hive version
```

The installer checks what it downloads and puts `hive` in `~/.hive/bin`
([Installing](../release.md#installing)). Ubuntu has `curl`. On Debian,
install it first: `sudo apt-get install -y curl`.

The same line works from PowerShell or the old command prompt, without
opening the Linux first:

```powershell
wsl sh -c "curl -fsSL https://hivefleet.dev/install.sh | sh"
```

Homebrew inside WSL works too:
`brew install lamplitlabs/tap/hivefleet` ([Homebrew](../release.md#homebrew)).

Run in Git Bash, the installer stops and names this page. Git Bash is a
Windows shell, and Hive needs the Linux.

## 4. Open the console in your Windows browser

In the Linux:

```bash
hive hub
```

It prints a line that starts `Serving on http://127.0.0.1:8765`, and then
`Could not open the browser`. That second line is expected: there is no
browser in the Linux. Open `http://127.0.0.1:8765` in your Windows browser
yourself. `http://localhost:8765` is the same page. Windows passes its own
`localhost` through to the Linux (Microsoft's `localhostForwarding`, which
is on unless you turned it off). Inside the Linux the console still listens
on `127.0.0.1` only.

From here on, [Set up your first project](first-project.md) is the guide
to follow, from its step 2.

## 5. Keep Hive running

Stop the `hive hub` you started (Ctrl-C), then, in the Linux:

```bash
hive service install     # the runner and the console, as two services
hive service status      # each service file, its state, the runner's heartbeat
```

The services belong to your Linux user and run under its `systemd`. They
start by themselves whenever you open the Linux, and are started again
when they exit ([Run Hive as two services](operations.md#1-run-hive-as-two-services)).

One thing is different from a Linux machine. **Windows stops a Linux that
nobody is using**, about 15 seconds after its last window closes, and Hive
stops with it. The store is safe, and when you open the Linux again the
services are back. A job whose run was cut off goes back in the queue
([The daemon loop](../runner.md#the-daemon-loop)). But no work is done
while the Linux is stopped.

To have Hive work with no window open, tell Windows not to stop an idle
Linux. Put these two lines in the file `.wslconfig` in your Windows user
folder (`C:\Users\YOU\.wslconfig`; make the file when it is not there):

```ini
[general]
instanceIdleTimeout=-1
```

Then run `wsl --shutdown` in PowerShell and open the Linux once. Now you
can close the window. The setting is Microsoft's
([Advanced settings](https://learn.microsoft.com/windows/wsl/wsl-config));
update WSL first when yours is old (`wsl --update`).

After Windows itself restarts, the Linux is not running until you open it
once. Hive starts with it.

## Check that it worked

- `hive version`, typed in the Linux, prints a version.
- `hive service status` says both services are `active (running)` and
  shows the runner's heartbeat.
- `http://127.0.0.1:8765` in your Windows browser shows the console.
- With the two lines of step 5 in place: close every WSL window, wait a
  minute, and load the console again. It still answers.

## When something goes wrong

- **The installer says `hive does not run on Windows itself yet`.** You ran
  it in Git Bash, MSYS or Cygwin. Run the line in the Linux (step 3).
- **`curl: command not found`.** The Linux has no `curl` (Debian's does
  not): `sudo apt-get install -y curl`.
- **The browser cannot open `127.0.0.1:8765`.** Ask the Linux whether the
  console is up: `hive service status`, or look at the terminal where
  `hive hub` runs. When it is up and Windows still cannot reach it, run
  `wsl --list --verbose` in PowerShell: under `VERSION` it must say `2`.
  Then look in `.wslconfig` for a line that turns `localhostForwarding`
  off.
- **The console was there and is gone.** Windows stopped the Linux (step
  5). `wsl --list --running` in PowerShell says whether it runs. Open the
  Linux again, and add the two lines of step 5 to keep it up.
- **`hive service install` fails, or the services are not there after you
  open the Linux again.** `ps -p 1 -o comm=` must print `systemd` (step 2).
- **`journalctl --user` shows lines `Invalid environment assignment`.**
  Under WSL your `PATH` carries the Windows one, and its folders have
  spaces in their names (`Program Files`). Hive up to v0.1.6 wrote that
  `PATH` into the service files in a form `systemd` reads only up to the
  first space. The services run all the same, because the Linux folders
  come first. A newer Hive writes it whole: update, then run
  `hive service install` again.
- **A job cannot find `git`, `gh` or a compiler.** Install it in the
  Linux (step 1).

## Read more

- [Releases and `hive update`](../release.md#windows): where each release
  is tried, WSL among them.
- [Keep Hive running, updated and backed up](operations.md): the services,
  updates and backups, the same in WSL as on any Linux.
- [Windows port](../windows-port.md): Hive on Windows itself, without WSL,
  and how far that is.
