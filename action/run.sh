#!/usr/bin/env bash
# Runs rlsspec with the action's inputs. The report goes to INPUT_OUTPUT when set, and the step fails on a
# non-zero exit only after it is written. DATABASE_URL comes from the job's env and is never printed.
set -euo pipefail

fail() {
  echo "::error title=rlsspec::$1"
  exit 1
}

case "$INPUT_COMMAND" in
  test | lint | cover) ;;
  *) fail "'command' must be test, lint or cover, got '${INPUT_COMMAND}'" ;;
esac

read -r -a extra <<< "${INPUT_ARGS:-}"
cmd=(rlsspec "$INPUT_COMMAND" -c "$INPUT_CONFIG" --format "$INPUT_FORMAT" ${extra[@]+"${extra[@]}"})

set +e
if [[ -z "$INPUT_OUTPUT" ]]; then
  "${cmd[@]}"
  code=$?
else
  mkdir -p "$(dirname "$INPUT_OUTPUT")"
  if [[ "$INPUT_FORMAT" == text ]]; then
    "${cmd[@]}" | tee "$INPUT_OUTPUT"
    code=${PIPESTATUS[0]}
  else
    "${cmd[@]}" > "$INPUT_OUTPUT"
    code=$?
    if [[ -s "$INPUT_OUTPUT" ]]; then
      echo "rlsspec ${INPUT_COMMAND}: ${INPUT_FORMAT} report written to ${INPUT_OUTPUT}"
    else
      # Config and connection errors print nothing on stdout: leave no empty report behind.
      rm -f "$INPUT_OUTPUT"
      echo "rlsspec ${INPUT_COMMAND}: no report, rlsspec stopped before producing one (see the error above)"
    fi
  fi
fi
set -e

echo "exit-code=${code}" >> "$GITHUB_OUTPUT"
if [[ "$code" -ne 0 ]]; then
  echo "::error title=rlsspec::rlsspec ${INPUT_COMMAND} exited with ${code}"
fi
exit "$code"
