#!/usr/bin/env bash
# Turn verify shots into images, and publish images for GitHub issues and PRs.
#
#   media.sh png SHOT.ansi...            render each shot to SHOT.png, all frames the same size
#   media.sh gif OUT.gif SHOT.ansi...    render the shots, then join them into one looping GIF
#   media.sh publish KEY FILE...         push FILEs to the `media` branch under KEY/,
#                                        print one Markdown image line per file
#
# KEY names the issue or PR, for example `pr-117` or `issue-42`.
# The links pin the commit, so a later publish never changes an old PR body.
# Needs: aha, chromium, magick (ImageMagick 7).
# MEDIA_REMOTE (default origin) and MEDIA_REPO (default: gh repo view) change the target.
set -euo pipefail

BG='#121211'
DELAY="${MEDIA_DELAY:-150}" # GIF frame time, in 1/100 s
REMOTE="${MEDIA_REMOTE:-origin}"

die() { echo "media: $*" >&2; exit 1; }

need() { for c in "$@"; do command -v "$c" >/dev/null || die "needs '$c' on PATH"; done; }

# One shot to one trimmed PNG next to it. Prints the PNG path.
render_one() {
  local ansi="$1" png="${1%.ansi}.png" html
  [[ -f $ansi ]] || die "no such shot: $ansi"
  html="$(mktemp --suffix=.html)"
  {
    printf '<!doctype html><meta charset=utf-8><style>'
    printf 'html,body{margin:0;background:%s;color:#ebdbb2}' "$BG"
    printf 'pre{margin:0;padding:0;font:14px/1.15 "JetBrains Mono","DejaVu Sans Mono",monospace;white-space:pre}'
    printf '</style><pre>'
    aha --no-header <"$ansi"
    printf '</pre>'
  } >"$html"
  # The window is larger than any terminal we drive; trim cuts it to the text.
  chromium --headless=new --disable-gpu --hide-scrollbars --window-size=2400,1600 \
    --screenshot="$png" "file://$html" >/dev/null 2>&1
  rm -f "$html"
  [[ -s $png ]] || die "chromium wrote no image for $ansi"
  magick "$png" -trim +repage "$png"
  echo "$png"
}

# Render all shots, then pad each to the largest one, so GIF frames do not jump.
cmd_png() {
  need aha chromium magick
  [[ $# -gt 0 ]] || die "png needs at least one SHOT.ansi"
  local pngs=() p w=0 h=0 pw ph
  for a in "$@"; do pngs+=("$(render_one "$a")"); done
  for p in "${pngs[@]}"; do
    read -r pw ph < <(magick identify -format '%w %h\n' "$p")
    (( pw > w )) && w=$pw; (( ph > h )) && h=$ph
  done
  for p in "${pngs[@]}"; do
    magick "$p" -background "$BG" -gravity northwest -extent "${w}x${h}" \
      -bordercolor "$BG" -border 16 "$p"
    echo "$p"
  done
}

cmd_gif() {
  local out="${1:?gif needs OUT.gif}"; shift
  [[ $out == *.gif ]] || die "OUT must end in .gif"
  local pngs; mapfile -t pngs < <(cmd_png "$@")
  magick -delay "$DELAY" -loop 0 "${pngs[@]}" -layers Optimize "$out"
  echo "$out"
}

cmd_publish() {
  local key="${1:?publish needs KEY}"; shift
  [[ $key =~ ^[a-z0-9][a-z0-9-]*$ ]] || die "KEY must be lowercase letters, digits, and '-', for example pr-117"
  [[ $# -gt 0 ]] || die "publish needs at least one FILE"
  local f; for f in "$@"; do [[ -f $f ]] || die "no such file: $f"; done
  local repo parent=() tree sha name blob
  repo="${MEDIA_REPO:-$(gh repo view --json nameWithOwner -q .nameWithOwner)}"
  # Build the commit with plumbing in a private index: no worktree, no local
  # branch, and the checkout you work in stays as it is.
  GIT_INDEX_FILE="$(mktemp -u)"; export GIT_INDEX_FILE
  if git ls-remote --exit-code "$REMOTE" refs/heads/media >/dev/null 2>&1; then
    git fetch -q "$REMOTE" refs/heads/media
    parent=(-p "$(git rev-parse FETCH_HEAD)")
    git read-tree FETCH_HEAD
  else
    git read-tree --empty
  fi
  for f in "$@"; do
    blob="$(git hash-object -w "$f")"
    git update-index --add --cacheinfo "100644,$blob,$key/$(basename "$f")"
  done
  tree="$(git write-tree)"
  rm -f "$GIT_INDEX_FILE"
  sha="$(git commit-tree "$tree" "${parent[@]}" -m "media: $key")"
  # The pre-push hook checks code; the media branch holds only images.
  git push -q --no-verify "$REMOTE" "$sha:refs/heads/media"
  for f in "$@"; do
    name="$(basename "$f")"
    echo "![${name%.*}](https://raw.githubusercontent.com/$repo/$sha/$key/$name)"
  done
}

sub="${1:-}"; shift || true
case "$sub" in
  png) cmd_png "$@" ;;
  gif) cmd_gif "$@" ;;
  publish) cmd_publish "$@" ;;
  *) sed -n '2,/^set -euo/p' "$0" | grep '^#'; exit 2 ;;
esac
