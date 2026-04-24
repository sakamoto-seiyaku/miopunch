package stunclient

import "testing"

func TestParseEndpoint(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name    string
		raw     string
		wantHP  string
		wantSch EndpointScheme
		wantErr bool
	}{
		{name: "dual", raw: "1.2.3.4:3478", wantHP: "1.2.3.4:3478", wantSch: EndpointSchemeDual},
		{name: "udp", raw: "udp://1.2.3.4:3478", wantHP: "1.2.3.4:3478", wantSch: EndpointSchemeUDP},
		{name: "tcp", raw: "tcp://example.com:3478", wantHP: "example.com:3478", wantSch: EndpointSchemeTCP},
		{name: "trim", raw: "  udp://1.2.3.4:3478  ", wantHP: "1.2.3.4:3478", wantSch: EndpointSchemeUDP},
		{name: "unsupported_scheme", raw: "http://1.2.3.4:3478", wantErr: true},
		{name: "unsupported_format", raw: "1.2.3.4:3478?x=y", wantErr: true},
		{name: "empty", raw: "   ", wantErr: true},
	}

	for _, tt := range tests {
		tt := tt
		t.Run(tt.name, func(t *testing.T) {
			t.Parallel()

			got, err := ParseEndpoint(tt.raw)
			if (err != nil) != tt.wantErr {
				t.Fatalf("ParseEndpoint(%q) error = %v, wantErr %v", tt.raw, err, tt.wantErr)
			}
			if tt.wantErr {
				return
			}
			if got.HostPort != tt.wantHP {
				t.Fatalf("ParseEndpoint(%q) hostport = %q, want %q", tt.raw, got.HostPort, tt.wantHP)
			}
			if got.Scheme != tt.wantSch {
				t.Fatalf("ParseEndpoint(%q) scheme = %q, want %q", tt.raw, got.Scheme, tt.wantSch)
			}
		})
	}
}

func TestFilterHostPorts(t *testing.T) {
	t.Parallel()

	usable, ignored, errs := FilterHostPorts([]string{
		"tcp://t.example:3478",
		"udp://u.example:3478",
		"d.example:3478",
		" ",
	}, EndpointSchemeUDP)

	if len(errs) != 1 {
		t.Fatalf("FilterHostPorts() errs = %v, want 1 error", errs)
	}
	if len(ignored) != 1 || ignored[0] != "tcp://t.example:3478" {
		t.Fatalf("FilterHostPorts() ignored = %v, want tcp endpoint ignored", ignored)
	}
	if len(usable) != 2 || usable[0] != "u.example:3478" || usable[1] != "d.example:3478" {
		t.Fatalf("FilterHostPorts() usable = %v, want udp:// stripped + dual kept", usable)
	}
}
