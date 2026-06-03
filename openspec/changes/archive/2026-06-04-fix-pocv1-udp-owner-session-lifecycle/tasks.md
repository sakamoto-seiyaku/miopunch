## 1. OpenSpec

- [x] 1.1 Add a change describing POC v1 UDP owner/session lifecycle semantics.
- [x] 1.2 Add delta specs for UDP owner, dial/punch, secure-session, and runtime behavior.

## 2. Implementation

- [x] 2.1 Make `PathResult.Close` stop closing the borrowed Runtime UDP socket.
- [x] 2.2 Make secure-session transport close paths stop closing the borrowed Runtime UDP socket.
- [x] 2.3 Remove transport-fatal peer sessions from Runtime before retrying future actions.

## 3. Tests and Notes

- [x] 3.1 Add focused tests proving `PathResult.Close` does not close the UDP socket.
- [x] 3.2 Add focused tests proving failed/closed secure sessions do not close the UDP socket.
- [x] 3.3 Update the Chinese smoke/debug note with the ownership root cause and fix.

## 4. Validation

- [x] 4.1 Run `go test ./dataplane ./internal/pocv1/punch ./internal/pocv1/session ./internal/pocv1/runtime -count=1`.
- [x] 4.2 Run `go test ./cmd/miopunch -count=1`.
