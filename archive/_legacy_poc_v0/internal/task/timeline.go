package task

import (
	"time"

	"github.com/miopunch/miopunch/internal/poc"
)

type TimelineEntry struct {
	At      time.Time
	Stage   poc.Stage
	Message string
}
