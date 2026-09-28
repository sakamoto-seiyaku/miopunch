package punchdecision

import (
	"github.com/miopunch/miopunch/internal/punching"
	"github.com/miopunch/miopunch/nat"
)

type NatFeature = nat.NatFeature

const (
	EasyNAT = nat.EasyNAT
	HardNAT = nat.HardNAT
)

var (
	DetectMode0 = punching.DetectMode0
	DetectMode1 = punching.DetectMode1
	DetectMode2 = punching.DetectMode2
	DetectMode3 = punching.DetectMode3
	DetectMode4 = punching.DetectMode4

	DetectRoleSender   = punching.DetectRoleSender
	DetectRoleReceiver = punching.DetectRoleReceiver
)

func ClassifyNATFeature(addresses, localIPs []string) (*NatFeature, error) {
	return nat.ClassifyNATFeature(addresses, localIPs)
}

func ClassifyFeatureCount(features []*NatFeature) (int, int, int) {
	return nat.ClassifyFeatureCount(features)
}
