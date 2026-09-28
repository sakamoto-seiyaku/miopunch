package http_panel

import (
	"encoding/json"
	"net/http"

	"github.com/miopunch/miopunch/internal/poc"
)

func writeJSON(w http.ResponseWriter, status int, value any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(value)
}

func writeError(w http.ResponseWriter, resp ErrorResponse) {
	status := poc.HTTPStatusFromExitCode(resp.ExitCode)
	writeJSON(w, status, resp)
}
