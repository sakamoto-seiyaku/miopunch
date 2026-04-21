package controlplane

import "encoding/base32"

var base32RawNoPad = base32.StdEncoding.WithPadding(base32.NoPadding)
