package punching

import (
	"fmt"
	"time"

	"github.com/miopunch/miopunch/internal/authutil"
)

func NewTransactionID() string {
	id, _ := authutil.RandID()
	return fmt.Sprintf("%d%s", time.Now().Unix(), id)
}
