#!/bin/sh
# A stand-in ACP agent for the lane tests: answers the four requests a
# chat turn sends and writes every session mode it is set to, one per
# line, into $FAKE_AGENT_LOG. It offers the mode ids Claude Code offers.
while IFS= read -r line; do
    id=$(printf '%s' "$line" | sed -n 's/.*"id":\("[^"]*"\|[0-9][0-9]*\).*/\1/p')
    [ -n "$id" ] || continue
    case "$line" in
        *'"method":"initialize"'*)
            result='{"protocolVersion":1,"agentCapabilities":{}}' ;;
        *'"method":"session/new"'*)
            result='{"sessionId":"fake-session","modes":{"currentModeId":"default","availableModes":[{"id":"default","name":"Default"},{"id":"acceptEdits","name":"Accept edits"},{"id":"plan","name":"Plan"},{"id":"bypassPermissions","name":"Bypass"}]}}' ;;
        *'"method":"session/set_mode"'*)
            printf '%s\n' "$line" | sed -n 's/.*"modeId":"\([^"]*\)".*/\1/p' >> "$FAKE_AGENT_LOG"
            result='{}' ;;
        *'"method":"session/prompt"'*)
            result='{"stopReason":"end_turn"}' ;;
        *)
            printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32601,"message":"method not found"}}\n' "$id"
            continue ;;
    esac
    printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$result"
done
