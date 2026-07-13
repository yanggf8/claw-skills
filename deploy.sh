#!/bin/sh
# deploy.sh — idempotently (re)create the nullclaw skill symlinks for THIS repo.
#
# For every skill dir in this repo that contains a SKILL.md, ensure
#   ~/.nullclaw/skills/<name>  is a symlink -> <repo>/<name>
# Also ensure the sibling lib symlink
#   ~/.nullclaw/skills/lib     -> <repo>/lib
# so scripts can import lib via ../../lib relative to run.py.
#
# Safety contract (nullclaw skills must never have delivery broken):
#   * This is a deploy/maintenance tool, NOT a pre-cron gate. It only touches
#     symlink bookkeeping under ~/.nullclaw/skills and never runs a skill.
#   * A deployed target that is a REAL directory (a drifted copy, e.g. one
#     `cp`'d in place of the symlink) is NEVER clobbered silently. Without
#     --force it is reported and skipped. With --force it is backed up
#     (mv to <name>.stale-copy.<ts>.bak) first, and only then replaced.
#   * A deployed target that is a symlink pointing OUTSIDE this repo is owned
#     by another repo (e.g. cct -> ~/a/cct/skills/cct, ainews -> ~/b/ainews).
#     It is left untouched even if this repo happens to have a same-named dir.
#   * Before replacing a copy, files present in the copy but absent from the
#     repo dir (copy-only content, e.g. inflation-con/config.json) are warned
#     about — those would be lost.
#
# Pure POSIX sh (runs under dash). No network. Exits 0 on success.

set -u

# --- config ---------------------------------------------------------------
REPO_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
SKILLS_DIR="${NULLCLAW_SKILLS_DIR:-$HOME/.nullclaw/skills}"
TS=$(date +%Y%m%d-%H%M%S)

FORCE=0
for arg in "$@"; do
	case "$arg" in
		--force) FORCE=1 ;;
		-h|--help)
			cat <<EOF
Usage: deploy.sh [--force]

Re-create ~/.nullclaw/skills symlinks for every SKILL.md-bearing dir in
$REPO_DIR (plus the sibling lib symlink).

  --force   Back up and replace deployed real-directory copies that shadow a
            repo skill (mv to <name>.stale-copy.<ts>.bak, then symlink).
            Without --force such copies are reported and skipped.
EOF
			exit 0
			;;
		*)
			printf '[WARN] unknown argument: %s (ignored)\n' "$arg" >&2
			;;
	esac
done

# --- counters (newline-delimited name lists for the summary) --------------
N_CREATED=0
N_OK=0
N_SKIPPED_COPY=0
N_SKIPPED_FOREIGN=0
N_BACKED_UP=0
N_LOSE_FILES=0
CREATED_LIST=""
OK_LIST=""
SKIPPED_COPY_LIST=""
SKIPPED_FOREIGN_LIST=""
BACKED_UP_LIST=""
LOSE_FILES_LIST=""

add() { # add <var-name> <item>  — append item to a newline list held in $var
	_av=$1
	_ai=$2
	eval "_cur=\$$_av"
	if [ -n "$_cur" ]; then
		eval "$_av=\"\$_cur
\$_ai\""
	else
		eval "$_av=\"\$_ai\""
	fi
}

# warn_copy_only <copy_dir> <repo_dir> <name>
# Prints any file under copy_dir that has no counterpart under repo_dir.
# Returns 0 if copy-only files were found, 1 otherwise.
warn_copy_only() {
	_copy=$1; _repo=$2; _name=$3
	_found=1
	# Enumerate every regular file / symlink in the copy, compare to repo.
	# `find ... -print` on relative paths keeps the comparison simple & POSIX.
	_rel_list=$(cd "$_copy" 2>/dev/null && find . \( -type f -o -type l \) -print 2>/dev/null)
	_oldifs=$IFS
	IFS='
'
	for _rel in $_rel_list; do
		[ -n "$_rel" ] || continue
		if [ ! -e "$_repo/$_rel" ] && [ ! -L "$_repo/$_rel" ]; then
			printf '  [WOULD LOSE] %s: %s\n' "$_name" "$_rel" >&2
			_found=0
		fi
	done
	IFS=$_oldifs
	return $_found
}

# link_one <name> <repo_target>
# Ensure $SKILLS_DIR/<name> is a symlink -> <repo_target>.
link_one() {
	name=$1
	target=$2
	link="$SKILLS_DIR/$name"

	# Already a symlink? Decide by where it resolves.
	if [ -L "$link" ]; then
		cur=$(readlink "$link")
		# Resolve to an absolute, canonical path for comparison.
		curreal=$(CDPATH= cd -- "$(dirname -- "$link")" 2>/dev/null && \
			cd -- "$(dirname -- "$cur")" 2>/dev/null && \
			printf '%s/%s' "$(pwd -P)" "$(basename -- "$cur")")
		want=$target

		if [ "$curreal" = "$want" ]; then
			N_OK=$((N_OK + 1)); add OK_LIST "$name"
			return 0
		fi

		# Symlink points somewhere else. If it points OUTSIDE this repo, it is
		# owned by another repo — leave it alone.
		case "$curreal" in
			"$REPO_DIR"/*)
				# Stale intra-repo symlink (e.g. wrong subpath) — safe to fix.
				rm -f "$link" && ln -s "$target" "$link"
				N_CREATED=$((N_CREATED + 1)); add CREATED_LIST "$name (relinked)"
				;;
			*)
				printf '[SKIP] %s: symlink points outside this repo (%s) — owned by another repo, left untouched\n' \
					"$name" "${curreal:-$cur}" >&2
				N_SKIPPED_FOREIGN=$((N_SKIPPED_FOREIGN + 1)); add SKIPPED_FOREIGN_LIST "$name"
				;;
		esac
		return 0
	fi

	# A real directory sitting where the symlink should be = a drifted copy.
	if [ -d "$link" ]; then
		if [ "$FORCE" -eq 1 ]; then
			# Warn about copy-only content that the replace would discard.
			if warn_copy_only "$link" "$target" "$name"; then
				N_LOSE_FILES=$((N_LOSE_FILES + 1)); add LOSE_FILES_LIST "$name"
			fi
			bak="$link.stale-copy.$TS.bak"
			if mv -- "$link" "$bak"; then
				printf '[BACKUP] %s: copy moved to %s\n' "$name" "$bak" >&2
				N_BACKED_UP=$((N_BACKED_UP + 1)); add BACKED_UP_LIST "$name"
				ln -s "$target" "$link"
				N_CREATED=$((N_CREATED + 1)); add CREATED_LIST "$name (replaced copy)"
			else
				printf '[ERROR] %s: could not back up copy, leaving it in place\n' "$name" >&2
				N_SKIPPED_COPY=$((N_SKIPPED_COPY + 1)); add SKIPPED_COPY_LIST "$name"
			fi
		else
			# Refuse to clobber. Still surface any copy-only content as a heads-up.
			if warn_copy_only "$link" "$target" "$name"; then
				N_LOSE_FILES=$((N_LOSE_FILES + 1)); add LOSE_FILES_LIST "$name"
			fi
			printf '[SKIP] %s: deployed target is a real directory (copy), not a symlink. Re-run with --force to back up and replace it.\n' \
				"$name" >&2
			N_SKIPPED_COPY=$((N_SKIPPED_COPY + 1)); add SKIPPED_COPY_LIST "$name"
		fi
		return 0
	fi

	# A non-directory, non-symlink file in the way (unexpected).
	if [ -e "$link" ]; then
		printf '[SKIP] %s: deployed target exists and is not a directory or symlink, left untouched\n' "$name" >&2
		N_SKIPPED_COPY=$((N_SKIPPED_COPY + 1)); add SKIPPED_COPY_LIST "$name"
		return 0
	fi

	# Nothing there — create the symlink.
	ln -s "$target" "$link"
	N_CREATED=$((N_CREATED + 1)); add CREATED_LIST "$name"
}

# --- main -----------------------------------------------------------------
if [ ! -d "$SKILLS_DIR" ]; then
	if ! mkdir -p "$SKILLS_DIR"; then
		printf '[ERROR] cannot create skills dir: %s\n' "$SKILLS_DIR" >&2
		exit 0
	fi
fi

# The sibling lib symlink (scripts import ../../lib relative to run.py).
if [ -d "$REPO_DIR/lib" ]; then
	link_one "lib" "$REPO_DIR/lib"
else
	printf '[WARN] no lib dir in repo (%s) — scripts will fail to import\n' "$REPO_DIR/lib" >&2
fi

# Every immediate subdir of the repo that contains a SKILL.md is a managed skill.
for d in "$REPO_DIR"/*/; do
	[ -d "$d" ] || continue
	[ -f "${d}SKILL.md" ] || continue
	name=$(basename -- "$d")
	target=${d%/}
	link_one "$name" "$target"
done

# --- summary --------------------------------------------------------------
printf '\n===== deploy summary =====\n'
printf 'repo:       %s\n' "$REPO_DIR"
printf 'skills dir: %s\n' "$SKILLS_DIR"
printf 'created/relinked : %d%s\n' "$N_CREATED" "${CREATED_LIST:+  [$(printf '%s' "$CREATED_LIST" | tr '\n' ',' | sed 's/,$//' | sed 's/,/, /g')]}"
printf 'already-correct  : %d%s\n' "$N_OK" "${OK_LIST:+  [$(printf '%s' "$OK_LIST" | tr '\n' ',' | sed 's/,$//' | sed 's/,/, /g')]}"
printf 'skipped-copy     : %d%s\n' "$N_SKIPPED_COPY" "${SKIPPED_COPY_LIST:+  [$(printf '%s' "$SKIPPED_COPY_LIST" | tr '\n' ',' | sed 's/,$//' | sed 's/,/, /g')]}"
printf 'skipped-foreign  : %d%s\n' "$N_SKIPPED_FOREIGN" "${SKIPPED_FOREIGN_LIST:+  [$(printf '%s' "$SKIPPED_FOREIGN_LIST" | tr '\n' ',' | sed 's/,$//' | sed 's/,/, /g')]}"
if [ "$N_BACKED_UP" -gt 0 ]; then
	printf 'backed-up        : %d  [%s]\n' "$N_BACKED_UP" "$(printf '%s' "$BACKED_UP_LIST" | tr '\n' ',' | sed 's/,$//' | sed 's/,/, /g')"
fi
if [ "$N_LOSE_FILES" -gt 0 ]; then
	printf 'WOULD-LOSE-FILES : %d  [%s]  (see [WOULD LOSE] lines above)\n' "$N_LOSE_FILES" "$(printf '%s' "$LOSE_FILES_LIST" | tr '\n' ',' | sed 's/,$//' | sed 's/,/, /g')"
fi
printf '==========================\n'

exit 0
