#!/usr/bin/env bash
# sb-agent-core — firma una release en borrador de un agente (Linux / Git Bash)
#
#   scripts/sign-release.sh [--publish] <owner/repo> <tag> [ruta-clave]
#
# La clave por defecto es ~/.sb-signing/sb-release-primary.priv, y su pública
# se busca al lado (mismo nombre con .pub). Nunca en un repo ni en GitHub.
#
# Descarga los .tar.gz/.zip del borrador, firma cada uno con zipsign usando
# como contexto el nombre del fichero (es lo que comprueba self_update: el
# file_name del archivo descargado, que es el nombre del asset), regenera su
# .sha256, verifica la firma y que el tar.gz sigue teniendo el binario en la
# raíz sin "./", y lo resube con --clobber. No publica salvo con --publish.
#
# Requiere: gh (autenticado), zipsign (`cargo install zipsign`), tar, sha256sum.
set -euo pipefail

PUBLISH=0
ARGS=()
for a in "$@"; do
  case "$a" in
    --publish) PUBLISH=1 ;;
    *) ARGS+=("$a") ;;
  esac
done
[[ ${#ARGS[@]} -ge 2 && ${#ARGS[@]} -le 3 ]] || {
  echo "Uso: $0 [--publish] <owner/repo> <tag> [ruta-clave]" >&2
  exit 2
}
REPO="${ARGS[0]}"
TAG="${ARGS[1]}"
KEY="${ARGS[2]:-$HOME/.sb-signing/sb-release-primary.priv}"
PUB="${KEY%.priv}.pub"

die() { echo "ERROR: $*" >&2; exit 1; }
[[ -f "$KEY" ]] || die "no existe la clave privada: $KEY"
[[ -f "$PUB" ]] || die "no existe la clave pública: $PUB"
command -v zipsign >/dev/null || die "falta zipsign (cargo install zipsign)"

# Con releases inmutables una release publicada ya no se puede re-subir.
[[ "$(gh release view "$TAG" -R "$REPO" --json isDraft -q .isDraft)" == "true" ]] \
  || die "$REPO $TAG no es un borrador"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir "$WORK/in" "$WORK/out" "$WORK/x"

gh release download "$TAG" -R "$REPO" -D "$WORK/in" -p '*.tar.gz' -p '*.zip'

shopt -s nullglob
signed=0
for f in "$WORK"/in/*.tar.gz "$WORK"/in/*.zip; do
  name="$(basename "$f")"
  out="$WORK/out/$name"
  case "$name" in
    *.tar.gz) kind=tar ;;
    *.zip)    kind=zip ;;
  esac

  # -f: zipsign 0.2.1 rechaza cualquier -o sin --force, aunque no exista.
  zipsign sign "$kind" -f -c "$name" -o "$out" "$f" "$KEY"
  zipsign verify "$kind" -q -c "$name" "$out" "$PUB"

  if [[ "$kind" == tar ]]; then
    # self_update busca el binario por ruta exacta: sin "./" y en la raíz
    # (ver release.yml). El binario es la entrada que, seguida de "-", es
    # prefijo del nombre del asset (oxipulse-x86_64-...tar.gz -> oxipulse).
    entries="$(tar -tzf "$out")"
    if grep -q '^\./' <<<"$entries"; then die "$name tiene entradas con prefijo ./"; fi
    bin=""
    while IFS= read -r e; do
      [[ "$name" == "$e-"* ]] && bin="$e"
    done <<<"$entries"
    [[ -n "$bin" ]] || die "$name no contiene el binario en la raíz"
    tar -xzf "$out" -C "$WORK/x" "$bin"
    [[ -s "$WORK/x/$bin" ]] || die "$name: no se pudo extraer $bin"
  fi

  hash="$(sha256sum <"$out" | cut -d' ' -f1)"
  printf '%s  %s\n' "$hash" "$name" >"$out.sha256"
  echo "firmado: $name"
  signed=$((signed + 1))
done
[[ $signed -gt 0 ]] || die "no hay .tar.gz ni .zip en $REPO $TAG"

gh release upload "$TAG" -R "$REPO" --clobber "$WORK"/out/*

PUBLISH_CMD=(gh release edit "$TAG" -R "$REPO" --draft=false)
if [[ $PUBLISH -eq 1 ]]; then
  "${PUBLISH_CMD[@]}"
  echo "publicada: $REPO $TAG"
else
  echo "Firmados $signed archivos. Para publicar:"
  echo "  ${PUBLISH_CMD[*]}"
fi
