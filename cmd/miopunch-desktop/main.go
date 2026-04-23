//go:build !desktop

package main

import (
	"fmt"
	"os"
)

func main() {
	fmt.Fprintln(os.Stderr, "miopunch-desktop is disabled in this build.")
	fmt.Fprintln(os.Stderr, "build with: go build -tags desktop ./cmd/miopunch-desktop")
	os.Exit(2)
}
