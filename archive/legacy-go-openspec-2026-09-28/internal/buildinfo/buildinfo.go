// Package buildinfo reports version metadata embedded into miopunch binaries.
package buildinfo

import "runtime/debug"

var releaseVersion string

// Version returns the release tag embedded by CI, the module version from Go
// build metadata, or a shortened VCS revision for development builds.
func Version() string {
	info, ok := debug.ReadBuildInfo()
	return versionFromBuildInfo(info, ok)
}

func versionFromBuildInfo(info *debug.BuildInfo, ok bool) string {
	if releaseVersion != "" {
		return releaseVersion
	}
	if !ok || info == nil {
		return "dev"
	}
	if info.Main.Version != "" && info.Main.Version != "(devel)" {
		return info.Main.Version
	}

	var revision string
	var modified string
	for _, s := range info.Settings {
		switch s.Key {
		case "vcs.revision":
			revision = s.Value
		case "vcs.modified":
			modified = s.Value
		}
	}
	if revision == "" {
		return "dev"
	}

	if len(revision) > 12 {
		revision = revision[:12]
	}
	if modified == "true" {
		return revision + "+dirty"
	}
	return revision
}
