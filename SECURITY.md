# Security policy

Hive runs coding agents on your own machines, with your credentials close
by. A hole in it matters, and reports are welcome.

## Reporting a vulnerability

Report it privately, through GitHub's private vulnerability reporting on
this repository:

1. Open <https://github.com/lamplitlabs/hive/security/advisories/new>, or on
   the repository page choose **Security**, then **Report a vulnerability**.
2. Describe what you found. Only the maintainers can read the report.

Do not open a public issue, a discussion or a pull request for a
vulnerability, and do not post the details anywhere public before a fix is
released.

## What to include

- What `hive version` prints (or `hive-hand --version`), and how you
  installed it: the installer, a release archive, or a build from source.
- Your operating system and chip.
- What an attacker can do, and what they need to start from: a web page you
  open, a repository an agent reads, a machine paired with yours, a local
  account on the same computer.
- The steps that show it, or a proof of concept.
- Whether anyone else knows.

Take real tokens, keys and private paths out of what you paste.

## What is in scope

- the `hive` binary: the runner, the harness, the store, every command;
- the console that `hive hub` serves;
- `hive-hand`, and the channel a machine uses to enrol and dial in;
- the installer, `scripts/install.sh`;
- the release workflow, `.github/workflows/release.yml`, and what it
  publishes.

A problem in a model provider, in GitHub or in a dependency belongs with
its own maintainers. Tell us as well when the way Hive uses it makes it
exploitable.

[Security notes](docs/security.md) says how Hive handles your code and your
data, and lists what is not built. A limit that page already states is not
a vulnerability. A way around a protection it describes is.

## Which versions get fixes

The latest release. A fix ships as a new release; older releases are not
patched.

## What happens next

A maintainer reads the report and answers on the advisory. When the fix is
released, the advisory is published with it, and you are credited unless
you ask not to be.
