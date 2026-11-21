#!/bin/sh
# The reference pages, generated from the program itself so that they cannot
# drift from it:
#
#   docs/commands.md  every command `hive help` lists, group by group: its
#                     one-line summary and the text `hive help COMMAND`
#                     prints (usage and flags)
#   docs/settings.md  every setting `hive settings list --json` reports, with
#                     its default and its description, one table per family
#
# Usage: scripts/gen-reference.sh           write both pages
#        scripts/gen-reference.sh --check   write nothing; say which committed
#                                           page differs from what the program
#                                           prints now, and exit 1
# Exit 0: written, or up to date; 1: --check found a difference; 2: usage, or
# the program could not be found, run or read.
#
# The program is the first of $HIVE_BIN, target/release/hive and
# target/debug/hive under the repository root, and `hive` on the PATH. It
# runs with HIVE_HOME set to a temporary home that is removed when the script
# ends, and with every other HIVE_* variable unset (HIVE_JOB, HIVE_AGENT,
# HIVE_SESSION, HIVE_PROJECT and the rest), so it never reads or writes a
# real store. Four commands are run and nothing else: `help`, `help COMMAND`,
# `init --store-only` (a store in the temporary home, to read the settings
# from) and `settings list --json`. No subcommand is run for its own help:
# a command that parses its own flags may act before it sees --help. The
# program refuses a store inside a git checkout, so TMPDIR must point
# outside every checkout.
#
# What changes in the program's text, so that the pages come out the same on
# every machine and every run: the temporary home reads ~/.hive and the
# user's home reads ~; colour codes, carriage returns and trailing spaces are
# dropped; tabs become spaces (the help indents with four spaces and a tab,
# which git reports as a whitespace error, and no page under docs/ holds a
# tab). In the prose and the tables, Markdown's own characters are escaped,
# what the program wrote in backticks stays code, and a bare web address
# becomes code so that no renderer turns an example into a link.
#
# Needs sh, awk, sed, grep, diff, cmp and mktemp. The awk programs keep to
# what every awk has (no gawk extension, no interval expression); the pages
# come out byte for byte the same under the one-true-awk, gawk and mawk.
set -eu

LC_ALL=C
export LC_ALL

me=gen-reference

die() {
  echo "$me: $*" >&2
  exit 2
}

usage() {
  echo "usage: $0 [--check]"
  echo "  writes docs/commands.md and docs/settings.md from what hive prints;"
  echo "  --check writes nothing and exits 1 when a committed page differs"
}

check=0
for arg in "$@"; do
  case $arg in
  --check) check=1 ;;
  -h | --help) usage; exit 0 ;;
  *) echo "$me: unknown argument $arg" >&2; usage >&2; exit 2 ;;
  esac
done

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
[ -d "$root/docs" ] || die "no docs directory under $root"

# The program: $HIVE_BIN, a build of this tree, then the PATH.
if [ -n "${HIVE_BIN:-}" ]; then
  case $HIVE_BIN in
  */*) bin=$HIVE_BIN ;;
  *) bin=$(command -v "$HIVE_BIN" 2> /dev/null) || bin=$HIVE_BIN ;;
  esac
  if [ ! -f "$bin" ] || [ ! -x "$bin" ]; then
    die "HIVE_BIN is $HIVE_BIN, which is not a program that can be run"
  fi
elif [ -x "$root/target/release/hive" ]; then
  bin=$root/target/release/hive
elif [ -x "$root/target/debug/hive" ]; then
  bin=$root/target/debug/hive
elif bin=$(command -v hive 2> /dev/null) && [ -n "$bin" ]; then
  :
else
  die "no hive to ask: set HIVE_BIN, build one (cargo build --release) or put hive on the PATH"
fi
case $bin in
/*) ;;
*) bin=$(pwd)/$bin ;;
esac

# A clean environment: no HIVE_* variable reaches the program but the
# temporary home set below.
unset HIVE_JOB HIVE_AGENT HIVE_SESSION HIVE_PROJECT
for name in $(env | sed -n 's/^\(HIVE_[A-Za-z0-9_]*\)=.*/\1/p'); do
  unset "$name"
done

tmpbase=${TMPDIR:-/tmp}
tmp=$(mktemp -d "${tmpbase%/}/hive-reference.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
trap 'exit 130' INT TERM HUP
case $tmp in
/*) ;;
*) tmp=$(pwd)/$tmp ;;
esac
tmp_real=$(cd "$tmp" && pwd -P)
home=$tmp/home
mkdir "$home" "$tmp/help" "$tmp/committed" "$tmp/generated"
home_real=$(cd "$home" && pwd -P)

HIVE_HOME=$home
NO_COLOR=1
export HIVE_HOME NO_COLOR

# The paths the pages must not carry, for the awk programs (through the
# environment, so that no backslash in them is read as an escape).
GEN_HOME=$home
GEN_HOME_REAL=$home_real
GEN_USER_HOME=${HOME:-}
GEN_USER_HOME=${GEN_USER_HOME%/}
GEN_HELP_DIR=$tmp/help
GEN_DOCS_LIST=$tmp/docs.list
GEN_SCRIPT=scripts/$(basename -- "$0")
export GEN_HOME GEN_HOME_REAL GEN_USER_HOME GEN_HELP_DIR GEN_DOCS_LIST GEN_SCRIPT

# Runs the program in the temporary directory, with nothing on stdin.
program() {
  (cd "$tmp" && exec "$bin" --no-color "$@") < /dev/null
}

# Says what the program wrote on stderr, then stops.
program_failed() {
  sed 's/^/  /' "$tmp/stderr" >&2
  die "$*"
}

# ---------------------------------------------------------------------------
# The awk programs.

# Shared: cleaning the program's text, and writing it as Markdown.
cat > "$tmp/lib.awk" << 'AWK'
BEGIN {
  BS = "\\"
  HOME_SET = ENVIRON["GEN_HOME"]
  HOME_REAL = ENVIRON["GEN_HOME_REAL"]
  USER_HOME = ENVIRON["GEN_USER_HOME"]
  SCRIPT = ENVIRON["GEN_SCRIPT"]
}

function fail(msg) {
  print "gen-reference: " msg | "cat 1>&2"
  close("cat 1>&2")
  failed = 1
  exit 2
}

# The character at position i, or nothing outside the string.
function at(s, i) {
  return (i < 1) ? "" : substr(s, i, 1)
}

function count(s, ch,    n, i) {
  n = 0
  for (i = 1; i <= length(s); i++)
    if (substr(s, i, 1) == ch)
      n++
  return n
}

function trim(s) {
  sub(/^[ \t\r]+/, "", s)
  sub(/[ \t\r]+$/, "", s)
  return s
}

# Every `from` in s becomes `to`, read as plain text.
function replace_all(s, from, to,    out, i) {
  if (from == "" || index(s, from) == 0)
    return s
  out = ""
  while ((i = index(s, from)) > 0) {
    out = out substr(s, 1, i - 1) to
    s = substr(s, i + length(from))
  }
  return out s
}

# The user's home directory becomes ~ where it stands as a whole path part.
function home_tilde(s,    out, i, next_char) {
  if (USER_HOME == "" || USER_HOME == "/" || index(s, USER_HOME) == 0)
    return s
  out = ""
  while ((i = index(s, USER_HOME)) > 0) {
    next_char = at(s, i + length(USER_HOME))
    if (next_char ~ /[A-Za-z0-9_.-]/)
      out = out substr(s, 1, i + length(USER_HOME) - 1)
    else
      out = out substr(s, 1, i - 1) "~"
    s = substr(s, i + length(USER_HOME))
  }
  return out s
}

# Tabs become spaces up to the next multiple of eight columns.
function expand_tabs(s,    out, i, c, col) {
  if (index(s, "\t") == 0)
    return s
  out = ""
  col = 0
  for (i = 1; i <= length(s); i++) {
    c = substr(s, i, 1)
    if (c == "\t") {
      out = out " "
      col++
      while (col % 8 != 0) {
        out = out " "
        col++
      }
    } else {
      out = out c
      # the bytes that continue a UTF-8 character take no column
      if (c < "\200" || c > "\277")
        col++
    }
  }
  return out
}

# One line of the program's output, made the same on every machine.
function clean(s) {
  gsub(/\r/, "", s)
  gsub(/\033\[[0-9;?]*[A-Za-z]/, "", s)
  s = replace_all(s, HOME_REAL, "~/.hive")
  s = replace_all(s, HOME_SET, "~/.hive")
  s = home_tilde(s)
  s = expand_tabs(s)
  sub(/[ ]+$/, "", s)
  return s
}

# s as a code span: the fence is longer than any run of backticks in s. In
# a table a pipe needs its backslash even inside code.
function code(s, table,    longest, run, i, c, fence, pad, out) {
  longest = 0
  run = 0
  out = ""
  for (i = 1; i <= length(s); i++) {
    c = substr(s, i, 1)
    if (c == "`") {
      run++
      if (run > longest)
        longest = run
    } else
      run = 0
    out = out ((table && c == "|") ? BS c : c)
  }
  fence = "`"
  for (i = 0; i < longest; i++)
    fence = fence "`"
  # Markdown drops one space at each end when both ends have one, so a
  # value that starts or ends with a backtick, or with a space at both ends,
  # gets a space more
  pad = ""
  if (at(s, 1) == "`" || at(s, length(s)) == "`" ||
      (at(s, 1) == " " && at(s, length(s)) == " " && s !~ /^ +$/))
    pad = " "
  return fence pad out pad fence
}

# The length of the web address that starts at position i of s, or 0. What
# ends it is a space or a backtick; punctuation that closes the sentence
# around it is left outside.
function url_length(s, i,    rest, scheme, n, tok, last) {
  if (at(s, i - 1) ~ /[A-Za-z0-9]/)
    return 0
  rest = substr(s, i)
  if (rest ~ /^https:\/\/[^ ]/)
    scheme = 8
  else if (rest ~ /^http:\/\/[^ ]/)
    scheme = 7
  else if (rest ~ /^www\.[A-Za-z0-9]/)
    scheme = 4
  else
    return 0
  n = match(rest, /[ `]/)
  tok = (n > 0) ? substr(rest, 1, n - 1) : rest
  while (length(tok) > scheme) {
    last = substr(tok, length(tok))
    if (index(",;:!?\"'*_~", last) > 0)
      tok = substr(tok, 1, length(tok) - 1)
    else if (last == "." && at(tok, length(tok) - 1) != ".")
      tok = substr(tok, 1, length(tok) - 1)
    else if (last == ")" && count(tok, ")") > count(tok, "("))
      tok = substr(tok, 1, length(tok) - 1)
    else if (last == "]" && count(tok, "]") > count(tok, "["))
      tok = substr(tok, 1, length(tok) - 1)
    else if (last == "}" && count(tok, "}") > count(tok, "{"))
      tok = substr(tok, 1, length(tok) - 1)
    else if (last == ">" && count(tok, ">") > count(tok, "<"))
      tok = substr(tok, 1, length(tok) - 1)
    else
      break
  }
  return (length(tok) > scheme) ? length(tok) : 0
}

# s with a backslash before every punctuation character, so that Markdown
# reads none of it as markup.
function literal(s,    out, i, c) {
  out = ""
  for (i = 1; i <= length(s); i++) {
    c = substr(s, i, 1)
    out = out ((index("!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~", c) > 0) ? BS c : c)
  }
  return out
}

# The number of backticks that start at position i of s.
function ticks(s, i,    n) {
  n = 0
  while (substr(s, i + n, 1) == "`")
    n++
  return n
}

# s as Markdown prose that reads exactly as the program wrote it. What the
# program put between backticks stays code (a run of backticks closes at the
# next run of the same length, as Markdown reads it), a bare web address
# becomes code (so that no renderer links an example), and every character
# Markdown would read as markup gets a backslash. An underscore inside a word
# is no markup and stays as it is. table: the text is a table cell, where a
# pipe needs a backslash too.
function text(s, table,    out, n, i, c, j, k, run, len, span) {
  out = ""
  n = length(s)
  i = 1
  while (i <= n) {
    c = substr(s, i, 1)
    if (c == "`") {
      run = ticks(s, i)
      j = i + run
      while (j <= n) {
        k = ticks(s, j)
        if (k == run)
          break
        j += (k > 0) ? k : 1
      }
      if (j > n) {
        # no run closes it: the backticks are text
        for (k = 0; k < run; k++)
          out = out BS c
        i += run
        continue
      }
      span = substr(s, i + run, j - i - run)
      if (table && index(span, BS "|") > 0)
        # code in a table cannot hold a backslash before a pipe
        out = out literal(substr(s, i, j + run - i))
      else {
        # Markdown drops one space at each end of code that has both
        if (length(span) > 2 && substr(span, 1, 1) == " " &&
            substr(span, length(span)) == " " && span !~ /^ +$/)
          span = substr(span, 2, length(span) - 2)
        out = out code(span, table)
      }
      i = j + run
      continue
    }
    if (c == "h" || c == "w") {
      len = url_length(s, i)
      if (len > 0) {
        out = out code(substr(s, i, len), table)
        i += len
        continue
      }
    }
    if (c == BS || index("*[]{}<>~$", c) > 0)
      out = out BS c
    else if (c == "|")
      out = out (table ? BS c : c)
    else if (c == "_")
      out = out ((at(s, i - 1) ~ /[A-Za-z0-9]/ && at(s, i + 1) ~ /[A-Za-z0-9]/) ? c : BS c)
    else if (c == "&")
      out = out ((substr(s, i) ~ /^&#?[A-Za-z0-9]+;/) ? BS c : c)
    else
      out = out c
    i++
  }
  return out
}

# A value as code. Code in a table cannot hold a backslash before a pipe;
# such a value is written out character by character instead.
function value_code(s, table) {
  if (table && index(s, BS "|") > 0)
    return literal(s)
  return code(s, table)
}

# A one-line summary as a sentence of its own paragraph: a capital on a
# plain first word, a full stop at the end, and nothing at the start that
# Markdown would read as a heading or a list.
function sentence(s,    word, after, last) {
  s = trim(s)
  if (s == "")
    return ""
  if (match(s, /^[a-z]+([-'][a-z]+)*/)) {
    word = RLENGTH
    after = at(s, word + 1)
    if (after == "" || after == " " ||
        (index(",:;", after) > 0 && (at(s, word + 2) == " " || at(s, word + 2) == "")))
      s = toupper(substr(s, 1, 1)) substr(s, 2)
  }
  last = substr(s, length(s))
  if (last != "." && last != "!" && last != "?")
    s = s "."
  s = text(s, 0)
  if (s ~ /^#/ || s ~ /^[-+] /)
    return BS s
  if (match(s, /^[0-9]+[.)] /))
    return substr(s, 1, RLENGTH - 2) BS substr(s, RLENGTH - 1)
  return s
}

# "a, b and c".
function join_and(list, n,    out, i) {
  out = ""
  for (i = 1; i <= n; i++)
    out = out ((i == 1) ? "" : ((i == n) ? " and " : ", ")) list[i]
  return out
}

function generated_note(source, where,    p) {
  p = "This page is generated by `" SCRIPT "` from what " source "."
  p = p " Do not edit it by hand: change the " where " in the code,"
  return p " then run the script again."
}
AWK

# `hive help` into a plan: the global flags (F), the variables they default
# to (V), each group's title (G) and each command's name, aliases and
# summary (C), tab-separated, in the program's order.
cat > "$tmp/listing.awk" << 'AWK'
{
  line = clean($0)
}
line ~ /^usage: / && flags == "" {
  rest = line
  while (match(rest, /--[a-z][a-z0-9-]*/)) {
    flags = flags ((flags == "") ? "" : " ") substr(rest, RSTART, RLENGTH)
    rest = substr(rest, RSTART + RLENGTH)
  }
  next
}
line ~ /^Global flags default to / {
  rest = line
  while (match(rest, /\$HIVE_[A-Z0-9_]+/)) {
    vars = vars ((vars == "") ? "" : " ") substr(rest, RSTART, RLENGTH)
    rest = substr(rest, RSTART + RLENGTH)
  }
  next
}
line ~ /^  [^ ]/ {
  if (title == "")
    fail("`hive help` lists a command before any group: " line)
  if (title != group) {
    group = title
    print "G\t" group
    groups++
  }
  row = substr(line, 3)
  name = row
  sub(/ .*$/, "", name)
  if (name !~ /^[a-z0-9][a-z0-9_-]*$/)
    fail("`hive help` lists a command with an unexpected name: " name)
  if (name in seen)
    fail("`hive help` lists " name " twice")
  seen[name] = 1
  rest = substr(row, length(name) + 1)
  aliases = ""
  if (rest ~ /^ \(/ && index(rest, ")") > 0) {
    aliases = substr(rest, 3, index(rest, ")") - 3)
    rest = substr(rest, index(rest, ")") + 1)
  }
  print "C\t" name "\t" aliases "\t" trim(rest)
  commands++
  next
}
line ~ /^[^ ]/ {
  title = line
}
END {
  if (failed)
    exit 2
  if (commands == 0)
    fail("`hive help` lists no command; is this hive?")
  if (flags == "" || vars == "")
    fail("`hive help` no longer names the global flags and their variables as before")
  print "F\t" flags
  print "V\t" vars
}
AWK

# The plan and each command's help into docs/commands.md.
cat > "$tmp/commands.awk" << 'AWK'
BEGIN {
  FS = "\t"
  HELP_DIR = ENVIRON["GEN_HELP_DIR"]
}
$1 == "F" { nflags = split($2, flags, " ") }
$1 == "V" { nvars = split($2, vars, " ") }
$1 == "G" { ngroups++; gtitle[ngroups] = $2 }
$1 == "C" { ncmds++; cgroup[ncmds] = ngroups; cname[ncmds] = $2; csummary[ncmds] = $4 }

# The program's help for one command, in a fenced block.
function block(name,    file, line, n, i, first, last, fence, run, longest, c, j) {
  file = HELP_DIR "/" name ".txt"
  n = 0
  longest = 0
  while ((getline line < file) > 0) {
    line = clean(line)
    lines[++n] = line
    run = 0
    for (j = 1; j <= length(line); j++) {
      c = substr(line, j, 1)
      if (c == "`") {
        run++
        if (run > longest)
          longest = run
      } else
        run = 0
    }
  }
  close(file)
  first = 1
  while (first <= n && lines[first] == "")
    first++
  last = n
  while (last >= first && lines[last] == "")
    last--
  if (last < first)
    fail("`hive help " name "` printed nothing")
  fence = "```"
  for (i = 3; i <= longest; i++)
    fence = fence "`"
  print fence "text"
  for (i = first; i <= last; i++)
    print lines[i]
  print fence
}

END {
  if (failed)
    exit 2
  for (i = 1; i <= nflags; i++) {
    known[flags[i]] = 1
    flag_code[i] = code(flags[i], 0)
  }
  for (i = 1; i <= nvars; i++) {
    flag = "--" tolower(substr(vars[i], 7))
    gsub(/_/, "-", flag)
    if (!(flag in known))
      fail("`hive help` names " vars[i] " but no flag " flag)
    var_flag[i] = code(flag, 0)
    var_code[i] = code(vars[i], 0)
  }

  print "# Commands"
  print ""
  p = "Every command has the same shape:"
  p = p " `hive [global flags] <command> [flags] [args]`."
  p = p " The global flags are " join_and(flag_code, nflags) "."
  p = p " The variables " join_and(var_code, nvars)
  p = p " set the defaults of " join_and(var_flag, nvars) ";"
  p = p " without `$HIVE_HOME` the store lives in `~/.hive`."
  print p
  print ""
  p = "Run `hive help` for this list and `hive help <command>`"
  p = p " for the text shown under each command."
  p = p " The help writes each flag with one dash (`-json`);"
  p = p " you can type two (`--json`), as the usage lines do."
  p = p " The settings have a page of their own: [Settings](settings.md)."
  print p
  print ""
  print generated_note("`hive help` and `hive help <command>` print", "help text")

  group = 0
  for (i = 1; i <= ncmds; i++) {
    if (cgroup[i] != group) {
      group = cgroup[i]
      print ""
      print "## " text(gtitle[group], 0)
    }
    print ""
    print "### " text(cname[i], 0)
    summary = sentence(csummary[i])
    if (summary != "") {
      print ""
      print summary
    }
    print ""
    block(cname[i])
  }
}
AWK

# The JSON array `hive settings list --json` prints into one line per
# setting: name, default, description, tab-separated. It reads any layout of
# that array, a line at a time (a JSON string never spans lines).
cat > "$tmp/json.awk" << 'AWK'
function hex(s,    v, i, d) {
  v = 0
  for (i = 1; i <= length(s); i++) {
    d = index("0123456789abcdef", tolower(substr(s, i, 1)))
    if (d == 0)
      return 65533
    v = v * 16 + d - 1
  }
  return v
}

function utf8(cp) {
  # control characters and the Unicode line separators
  if (cp < 32 || cp == 127 || cp == 133 || cp == 8232 || cp == 8233)
    return " "
  if (cp < 128)
    return sprintf("%c", cp)
  if (cp < 2048)
    return sprintf("%c%c", 192 + int(cp / 64), 128 + cp % 64)
  if (cp < 65536)
    return sprintf("%c%c%c", 224 + int(cp / 4096), 128 + int(cp / 64) % 64, 128 + cp % 64)
  return sprintf("%c%c%c%c", 240 + int(cp / 262144), 128 + int(cp / 4096) % 64,
                 128 + int(cp / 64) % 64, 128 + cp % 64)
}

# A JSON string's text. A line break or a tab in it becomes a space: the
# text goes on one line of a table.
function unescape(s,    out, n, i, c, e, cp, low) {
  out = ""
  n = length(s)
  i = 1
  while (i <= n) {
    c = substr(s, i, 1)
    if (c != BS) {
      out = out c
      i++
      continue
    }
    e = substr(s, i + 1, 1)
    if (e == "u") {
      cp = hex(substr(s, i + 2, 4))
      i += 6
      if (cp >= 55296 && cp <= 56319 && substr(s, i, 2) == BS "u") {
        low = hex(substr(s, i + 2, 4))
        if (low >= 56320 && low <= 57343) {
          cp = 65536 + (cp - 55296) * 1024 + (low - 56320)
          i += 6
        }
      }
      out = out utf8(cp)
      continue
    }
    if (e == "n" || e == "r" || e == "t" || e == "b" || e == "f")
      out = out " "
    else
      out = out e
    i += 2
  }
  out = clean(out)
  gsub(/[\001-\037\177]/, " ", out)
  return out
}

function emit() {
  if (!("name" in rec) || rec["name"] == "")
    fail("`hive settings list --json` printed a setting without a name")
  if (rec["name"] !~ /^[A-Za-z0-9][A-Za-z0-9_.-]*$/)
    fail("`hive settings list --json` printed a setting with an unexpected name: " rec["name"])
  print rec["name"] "\t" rec["default"] "\t" trim(rec["description"])
  settings++
}

{
  line = $0
  n = length(line)
  i = 1
  while (i <= n) {
    c = substr(line, i, 1)
    if (c == "\"") {
      j = i + 1
      while (j <= n) {
        d = substr(line, j, 1)
        if (d == BS) {
          j += 2
          continue
        }
        if (d == "\"")
          break
        j++
      }
      if (j > n)
        fail("`hive settings list --json` printed a string that does not end on its line")
      if (depth == 2 && is_object[depth]) {
        string = (j > i + 1) ? unescape(substr(line, i + 1, j - i - 1)) : ""
        if (want_value) {
          rec[key] = string
          want_value = 0
        } else
          key = string
      }
      i = j + 1
      continue
    }
    if (c == "{" || c == "[") {
      depth++
      is_object[depth] = (c == "{")
      if (depth == 1 && c != "[")
        fail("`hive settings list --json` no longer prints a list")
      if (depth == 2 && c == "{") {
        rec["name"] = ""
        rec["default"] = ""
        rec["description"] = ""
        want_value = 0
      }
    } else if (c == "}" || c == "]") {
      if (depth == 2 && c == "}")
        emit()
      depth--
      if (depth == 2)
        want_value = 0
    } else if (c == ":") {
      if (depth == 2)
        want_value = 1
    } else if (c == ",") {
      if (depth == 2)
        want_value = 0
    }
    i++
  }
}

END {
  if (failed)
    exit 2
  if (depth != 0)
    fail("`hive settings list --json` printed a list that does not close")
  if (settings == 0)
    fail("`hive settings list --json` printed no setting")
}
AWK

# The settings into docs/settings.md: one table per family, the families in
# alphabetical order, the settings without a family first.
cat > "$tmp/settings.awk" << 'AWK'
BEGIN {
  FS = "\t"
  DOCS_LIST = ENVIRON["GEN_DOCS_LIST"]
}
{
  n++
  name[n] = $1
  value[n] = $2
  about[n] = $3
  if ($1 in seen)
    fail("`hive settings list --json` printed " $1 " twice")
  seen[$1] = 1
}

# Whether line names the setting as a whole word: not as a part of a longer
# name such as daemon.sweep_keep_days for daemon.sweep.
function names(line, word,    i, rest, before, after, offset) {
  offset = 0
  rest = line
  while ((i = index(rest, word)) > 0) {
    before = at(line, offset + i - 1)
    after = substr(line, offset + i + length(word), 2)
    if (before !~ /[A-Za-z0-9_.-]/ && after !~ /^[A-Za-z0-9_]/ && after !~ /^[.-][A-Za-z0-9_]/)
      return 1
    offset += i
    rest = substr(rest, i + 1)
  }
  return 0
}

# For a setting the program does not describe: the pages under docs/ that
# name it, rather than a description made up here.
function pages(word,    file, line, found, base, out) {
  out = ""
  while ((getline file < DOCS_LIST) > 0) {
    found = 0
    while ((getline line < file) > 0)
      if (names(line, word)) {
        found = 1
        break
      }
    close(file)
    if (found) {
      base = file
      sub(/^.*\//, "", base)
      out = out ((out == "") ? "" : ", ") "[" text(base, 1) "](" base ")"
    }
  }
  close(DOCS_LIST)
  if (out == "")
    return "The program gives no description, and no page under `docs/` names this setting."
  return "The program gives no description. Named in " out "."
}

END {
  if (failed)
    exit 2
  for (i = 1; i <= n; i++) {
    dot = index(name[i], ".")
    if (dot > 1)
      is_family[substr(name[i], 1, dot - 1)] = 1
  }
  for (i = 1; i <= n; i++) {
    dot = index(name[i], ".")
    if (dot > 1)
      family[i] = substr(name[i], 1, dot - 1)
    else if (name[i] in is_family)
      family[i] = name[i]
    else
      family[i] = ""
    # a tab sorts before every character a name holds, so the settings
    # without a family come first and a family's own name leads its table
    key[i] = family[i] "\t" name[i]
    order[i] = i
  }
  for (i = 2; i <= n; i++) {
    moving = order[i]
    j = i - 1
    while (j >= 1 && key[order[j]] > key[moving]) {
      order[j + 1] = order[j]
      j--
    }
    order[j + 1] = moving
  }

  print "# Settings"
  print ""
  p = "Hive keeps its settings in the store."
  p = p " Run `hive settings` to read every setting with its value,"
  p = p " or `hive settings get NAME` to read one."
  p = p " To change one, run `hive settings set NAME VALUE`"
  p = p " or use the console's [Settings](console.md#settings) page;"
  p = p " a value the setting does not take is refused with the reason."
  p = p " The command has its own entry: [`settings`](commands.md#settings)."
  print p
  print ""
  p = generated_note("`hive settings list --json` prints", "description")
  p = p " That list also carries each setting's rule"
  p = p " (`kind`, `min`, `choices`), which this page leaves out."
  print p
  print ""
  p = "Each section below is one family:"
  p = p " the settings that share the part of their name before the first dot."
  p = p " The families come in alphabetical order."
  p = p " General comes first and holds the settings that belong to no family."
  p = p " A default of empty means the setting has no value until you set one."
  print p

  current = "\t"
  for (k = 1; k <= n; k++) {
    i = order[k]
    if (family[i] != current) {
      current = family[i]
      print ""
      print "## " ((current == "") ? "General" : text(current, 0))
      print ""
      print "| Setting | Default | What it does |"
      print "| --- | --- | --- |"
    }
    shown = (value[i] == "") ? "empty" : value_code(value[i], 1)
    does = (about[i] == "") ? pages(name[i]) : text(about[i], 1)
    print "| " code(name[i], 1) " | " shown " | " does " |"
  }
}
AWK

# ---------------------------------------------------------------------------
# The commands.

if ! program help > "$tmp/help.txt" 2> "$tmp/stderr"; then
  program_failed "$bin help failed"
fi
awk -f "$tmp/lib.awk" -f "$tmp/listing.awk" "$tmp/help.txt" > "$tmp/plan.tsv" ||
  die "could not read what $bin help prints"

awk -F '\t' '$1 == "C" { print $2 }' "$tmp/plan.tsv" > "$tmp/names"
while IFS= read -r name; do
  if ! program help "$name" > "$tmp/help/$name.txt" 2> "$tmp/stderr"; then
    program_failed "$bin help $name failed"
  fi
done < "$tmp/names"
awk -f "$tmp/lib.awk" -f "$tmp/commands.awk" "$tmp/plan.tsv" > "$tmp/generated/commands.md" ||
  die "could not write the commands page"

# ---------------------------------------------------------------------------
# The settings.

if ! program init --store-only > "$tmp/stdout" 2> "$tmp/stderr"; then
  sed 's/^/  /' "$tmp/stderr" >&2
  echo "$me: the temporary home is $home; TMPDIR must be outside every git checkout" >&2
  die "$bin init --store-only could not create a store to read the settings from"
fi
if ! program settings list --json > "$tmp/settings.json" 2> "$tmp/stderr"; then
  program_failed "$bin settings list --json failed"
fi
awk -f "$tmp/lib.awk" -f "$tmp/json.awk" "$tmp/settings.json" > "$tmp/settings.tsv" ||
  die "could not read what $bin settings list --json prints"

# The pages a setting without a description is looked up in.
: > "$tmp/docs.list"
for page in "$root"/docs/*.md; do
  case $page in
  */commands.md | */settings.md) continue ;;
  esac
  if [ -f "$page" ]; then
    printf '%s\n' "$page" >> "$tmp/docs.list"
  fi
done
awk -f "$tmp/lib.awk" -f "$tmp/settings.awk" "$tmp/settings.tsv" > "$tmp/generated/settings.md" ||
  die "could not write the settings page"

# ---------------------------------------------------------------------------
# Nothing of this machine or this run may be left in a page.

for page in commands settings; do
  for left in "$tmp" "$tmp_real"; do
    if grep -F -q -e "$left" "$tmp/generated/$page.md"; then
      die "docs/$page.md would carry the temporary path $left; nothing written"
    fi
  done
done

n_commands=$(awk 'END { print NR }' "$tmp/names")
n_groups=$(awk -F '\t' '$1 == "G" { n++ } END { print n + 0 }' "$tmp/plan.tsv")
n_settings=$(awk 'END { print NR }' "$tmp/settings.tsv")
n_tables=$(awk '/^## / { n++ } END { print n + 0 }' "$tmp/generated/settings.md")
summary="$n_commands commands in $n_groups groups, $n_settings settings in $n_tables tables"

# ---------------------------------------------------------------------------
# Compare, or write.

if [ "$check" -eq 1 ]; then
  stale=0
  for page in commands settings; do
    if [ ! -f "$root/docs/$page.md" ]; then
      echo "$me: docs/$page.md is missing" >&2
      stale=1
    elif ! cmp -s "$root/docs/$page.md" "$tmp/generated/$page.md"; then
      echo "$me: docs/$page.md differs from what $bin prints now:" >&2
      cp "$root/docs/$page.md" "$tmp/committed/$page.md"
      (cd "$tmp" && diff -u "committed/$page.md" "generated/$page.md" || true) |
        sed -e 's/^/  /' -e 40q >&2
      stale=1
    fi
  done
  if [ "$stale" -ne 0 ]; then
    echo "$me: run $GEN_SCRIPT and commit the pages" >&2
    exit 1
  fi
  echo "$me: docs/commands.md and docs/settings.md are what $bin prints ($summary)"
  exit 0
fi

for page in commands settings; do
  if [ -f "$root/docs/$page.md" ] && cmp -s "$root/docs/$page.md" "$tmp/generated/$page.md"; then
    echo "$me: docs/$page.md is unchanged"
  else
    cp "$tmp/generated/$page.md" "$root/docs/$page.md"
    echo "$me: wrote docs/$page.md"
  fi
done
echo "$me: $summary, from $bin"
