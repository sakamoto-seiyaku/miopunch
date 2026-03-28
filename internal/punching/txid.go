package punching

import (
	"fmt"
	"time"

	"github.com/miopunch/miopunch/xtcp/util/util"
)

func NewTransactionID() string {
	id, _ := util.RandID()
	return fmt.Sprintf("%d%s", time.Now().Unix(), id)
}
