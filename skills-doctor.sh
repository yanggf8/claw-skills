#!/bin/sh
# skills-doctor.sh — audit nullclaw skills-dir drift, READ-ONLY.
#
# Classifies every entry in the nullclaw skills dir against this repo:
#   OK        symlink whose real path is inside this repo
#   DRIFT     a real dir (copy) whose name matches a repo skill dir
#             (a symlink conversion would replace it — and may delete files)
#   info      unmanaged: symlink pointing outside this repo, or a copy with
#             no matching repo dir (e.g. autocli, cct, ainews, *.bak)
#
# For each DRIFT copy it also lists COPY-ONLY files (present in the deployed
# copy but not in the repo dir, ignoring __pycache__ and *.pyc). Those are the
# files a blind symlink conversion would LOSE — prefixed 'WOULD-LOSE:'.
#
# WARNING-ONLY. It never writes, never deletes, never converts, and ALWAYS
# exits 0. Safe to run next to cron: it only prints findings.

# Resolve repo root = the dir this script lives in.
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
REPO=${script_dir}

# Skills dir to audit (override with SKILLS_DIR for testing).
SKILLS_DIR=${SKILLS_DIR:-${HOME}/.nullclaw/skills}

# Real path of the repo root, for prefix comparison.
REPO_REAL=$(readlink -f -- "$REPO" 2>/dev/null || echo "$REPO")

drift_count=0
lose_count=0

if [ ! -d "$SKILLS_DIR" ]; then
	echo "INFO: skills dir not found: $SKILLS_DIR (nothing to audit)"
	exit 0
fi

# List copy-only files: present under $1 (deployed copy) but not under $2
# (repo dir), excluding __pycache__ dirs and *.pyc. Paths printed repo-relative
# with a leading './'. Uses temp files under a private, cleaned-up dir.
list_copy_only() {
	_dep=$1
	_repo=$2
	_tmp=$(mktemp -d 2>/dev/null) || return 0
	( CDPATH= cd -- "$_dep" 2>/dev/null && \
	  find . -type f ! -path '*/__pycache__/*' ! -name '*.pyc' 2>/dev/null | LC_ALL=C sort
	) >"$_tmp/dep" 2>/dev/null
	( CDPATH= cd -- "$_repo" 2>/dev/null && \
	  find . -type f ! -path '*/__pycache__/*' ! -name '*.pyc' 2>/dev/null | LC_ALL=C sort
	) >"$_tmp/repo" 2>/dev/null
	# lines in dep not in repo
	comm -23 "$_tmp/dep" "$_tmp/repo" 2>/dev/null
	rm -rf "$_tmp" 2>/dev/null
}

echo "skills-doctor: auditing $SKILLS_DIR against repo $REPO"
echo "-----------------------------------------------------------------"

for entry in "$SKILLS_DIR"/*; do
	# Guard: no matches (literal glob) or vanished entry.
	[ -e "$entry" ] || [ -L "$entry" ] || continue
	name=$(basename -- "$entry")

	# Skip the shared lib symlink and non-skill bookkeeping files quietly-ish.
	if [ "$name" = "lib" ]; then
		if [ -L "$entry" ]; then
			echo "OK:   lib (shared lib symlink)"
		else
			echo "info: lib is a COPY, not a symlink (expected symlink into repo)"
		fi
		continue
	fi

	if [ -L "$entry" ]; then
		# Symlink: is its real target inside this repo?
		target_real=$(readlink -f -- "$entry" 2>/dev/null)
		case "${target_real}/" in
			"$REPO_REAL"/*)
				echo "OK:   $name -> ${target_real#"$REPO_REAL"/} (symlink into repo)"
				;;
			*)
				echo "info: $name is an unmanaged symlink -> ${target_real:-<broken>} (outside this repo)"
				;;
		esac
		continue
	fi

	if [ -d "$entry" ]; then
		# Real directory (a copy). Drift iff repo has a same-named dir.
		if [ -d "$REPO/$name" ]; then
			echo "DRIFT: $name is a COPY of a repo skill (repo has $name/)"
			drift_count=$((drift_count + 1))
			# Copy-only files a symlink conversion would delete.
			co=$(list_copy_only "$entry" "$REPO/$name")
			if [ -n "$co" ]; then
				echo "$co" | while IFS= read -r f; do
					[ -n "$f" ] || continue
					echo "WOULD-LOSE: $name/${f#./}"
				done
				# count (subshell can't mutate lose_count; recount below)
			fi
		else
			echo "info: $name is an unmanaged copy (no $name/ in repo)"
		fi
		continue
	fi

	# Anything else (regular file, socket, etc.)
	echo "info: $name is not a dir or symlink (skipped)"
done

# Recompute WOULD-LOSE total for the summary (the per-entry loop counted DRIFTs
# in-process, but WOULD-LOSE lines were emitted from a subshell pipe).
lose_count=0
for entry in "$SKILLS_DIR"/*; do
	[ -d "$entry" ] || continue
	[ -L "$entry" ] && continue
	name=$(basename -- "$entry")
	[ "$name" = "lib" ] && continue
	[ -d "$REPO/$name" ] || continue
	co=$(list_copy_only "$entry" "$REPO/$name")
	[ -n "$co" ] || continue
	n=$(printf '%s\n' "$co" | grep -c .)
	lose_count=$((lose_count + n))
done

echo "-----------------------------------------------------------------"
echo "summary: ${drift_count} drift(ed) copy skill(s), ${lose_count} copy-only file(s) at risk"
if [ "$drift_count" -gt 0 ]; then
	echo "note: DRIFT skills run stale code; WOULD-LOSE files must be preserved before any symlink conversion."
fi
echo "(read-only audit — nothing was changed)"

# HARD CONTRACT: warning-only, never abort anything.
exit 0
