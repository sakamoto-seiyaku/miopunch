package localapi

import (
	"net/http"

	"github.com/miopunch/miopunch/internal/poc"
)

// RequireLocalAPIHost enforces the fixed Host header for LocalAPI requests.
func RequireLocalAPIHost(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Host != poc.LocalAPIHost {
			reqID, _ := poc.NewRequestID()
			writeError(w, ErrorResponse{
				Stage:      "localapi",
				ReasonCode: poc.ReasonCodeBadRequest,
				ExitCode:   poc.ExitCodeBadRequest,
				Message:    "invalid host for localapi request",
				Facts: []poc.Fact{
					{TermID: "localapi_host", Message: "host=" + r.Host},
					{TermID: "localapi_host_want", Message: "want_host=" + poc.LocalAPIHost},
				},
				Suggestions: []poc.Suggestion{
					{Message: "retry with Host: " + poc.LocalAPIHost},
				},
				RequestID: reqID,
			})
			return
		}
		next.ServeHTTP(w, r)
	})
}
