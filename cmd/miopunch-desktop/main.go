//go:build !desktop

package main

import (
	"fmt"
	"os"
)

func main() {
	fmt.Fprintln(os.Stderr, "miopunch-desktop is disabled in this build.")
	fmt.Fprintln(os.Stderr, "run locally with:")
	fmt.Fprintln(os.Stderr, "  go run -tags desktop,production ./cmd/miopunch-desktop")
	fmt.Fprintln(os.Stderr, "build release packages with:")
	fmt.Fprintln(os.Stderr, "  go build -tags desktop,production ./cmd/miopunch-desktop")
	os.Exit(2)
}
