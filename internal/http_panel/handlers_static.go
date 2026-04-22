package http_panel

import (
	"net/http"
)

func (s *Server) handleIndex(w http.ResponseWriter, r *http.Request) {
	u := *r.URL
	u.Path = "/index.html"

	r2 := r.Clone(r.Context())
	r2.URL = &u
	s.static.ServeHTTP(w, r2)
}
