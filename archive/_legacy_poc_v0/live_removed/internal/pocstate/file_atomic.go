package pocstate

import (
	"os"

	"github.com/miopunch/miopunch/internal/atomicfile"
)

func writeFileAtomic(path string, data []byte, perm os.FileMode) error {
	return atomicfile.WriteFile(path, data, perm)
}
