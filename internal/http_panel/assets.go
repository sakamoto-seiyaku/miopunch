package http_panel

import (
	"embed"
	"io/fs"
	"net/http"
)

//go:embed assets
var embeddedAssets embed.FS

func assetsFileServer() http.Handler {
	sub, err := fs.Sub(embeddedAssets, "assets")
	if err != nil {
		return http.NotFoundHandler()
	}
	return http.FileServer(http.FS(sub))
}
