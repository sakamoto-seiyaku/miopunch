package controlplane

import (
	"encoding/base32"
	"encoding/base64"
)

var base32RawNoPad = base32.StdEncoding.WithPadding(base32.NoPadding)

var base64URLNoPad = base64.RawURLEncoding
