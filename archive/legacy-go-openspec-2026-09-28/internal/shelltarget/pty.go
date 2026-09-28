package shelltarget

import "io"

// PTY is an interactive terminal session backed by a (Con)PTY.
// Implementations must be safe for a single reader and a single writer.
type PTY interface {
	io.ReadWriteCloser
	Resize(cols, rows int) error
	Wait() error
}
