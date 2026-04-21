package shellproto

import "io"

type Reader struct {
	r io.Reader
}

func NewReader(r io.Reader) *Reader {
	return &Reader{r: r}
}

func (r *Reader) ReadFrame() (Kind, []byte, error) {
	if r == nil {
		return 0, nil, io.ErrClosedPipe
	}
	return ReadFrame(r.r)
}

func (r *Reader) ReadJSON(out any) error {
	if r == nil {
		return io.ErrClosedPipe
	}
	return ReadJSON(r.r, out)
}

type Writer struct {
	w io.Writer
}

func NewWriter(w io.Writer) *Writer {
	return &Writer{w: w}
}

func (w *Writer) WriteFrame(kind Kind, payload []byte) error {
	if w == nil {
		return io.ErrClosedPipe
	}
	return WriteFrame(w.w, kind, payload)
}

func (w *Writer) WriteJSON(v any) error {
	if w == nil {
		return io.ErrClosedPipe
	}
	return WriteJSON(w.w, v)
}
