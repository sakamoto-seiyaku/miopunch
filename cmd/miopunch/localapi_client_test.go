package main

import (
	"context"
	"errors"
	"os"
	"testing"

	"github.com/miopunch/miopunch/internal/localapi"
	"github.com/miopunch/miopunch/internal/poc"
)

func TestConnectLocalAPIWithDeps_BootstrapSuccess(t *testing.T) {
	t.Parallel()

	userAddr := localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/user.sock"}
	wantClient := &localapi.Client{}

	bootstrapped := false
	client, gotAddr, err := connectLocalAPIWithDeps(context.Background(), "", localAPIConnectorDeps{
		currentOperatorSID: func() (string, error) { return "sid-1", nil },
		defaultSystemAddr:  func(string) (localapi.Addr, error) { return localapi.Addr{}, errors.New("system unavailable") },
		defaultUserAddr:    func(string) (localapi.Addr, error) { return userAddr, nil },
		bundleProbeAddr:    noBundleLocalAPIProbeAddr,
		probe: func(_ context.Context, addr localapi.Addr) (*localapi.Client, error) {
			if addr == userAddr && bootstrapped {
				return wantClient, nil
			}
			return nil, errors.New("dial failed")
		},
		bootstrap: func(addr localapi.Addr) error {
			if addr != userAddr {
				t.Fatalf("bootstrap(addr=%v), want %v", addr, userAddr)
			}
			bootstrapped = true
			return nil
		},
	})
	if err != nil {
		t.Fatalf("connectLocalAPIWithDeps() error = %v, want nil", err)
	}
	if client != wantClient {
		t.Fatalf("connectLocalAPIWithDeps() client = %p, want %p", client, wantClient)
	}
	if gotAddr != userAddr {
		t.Fatalf("connectLocalAPIWithDeps() addr = %v, want %v", gotAddr, userAddr)
	}
}

func TestConnectLocalAPIWithDeps_BootstrapStartFailureReported(t *testing.T) {
	t.Parallel()

	userAddr := localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/user.sock"}

	_, _, err := connectLocalAPIWithDeps(context.Background(), "", localAPIConnectorDeps{
		currentOperatorSID: func() (string, error) { return "sid-1", nil },
		defaultSystemAddr:  func(string) (localapi.Addr, error) { return localapi.Addr{}, errors.New("system unavailable") },
		defaultUserAddr:    func(string) (localapi.Addr, error) { return userAddr, nil },
		bundleProbeAddr:    noBundleLocalAPIProbeAddr,
		probe: func(context.Context, localapi.Addr) (*localapi.Client, error) {
			return nil, errors.New("dial failed")
		},
		bootstrap: func(addr localapi.Addr) error {
			if addr != userAddr {
				t.Fatalf("bootstrap(addr=%v), want %v", addr, userAddr)
			}
			return errors.New("failed to start daemon")
		},
	})

	failure := requireLocalAPIConnectionFailure(t, err)
	if failure.ReasonCode != poc.ReasonCodeDaemonNotRunning {
		t.Fatalf("failure.ReasonCode = %q, want %q", failure.ReasonCode, poc.ReasonCodeDaemonNotRunning)
	}
	if !hasFact(failure.Facts, "bootstrap_addr="+userAddr.String()) {
		t.Fatalf("failure.Facts missing bootstrap_addr, got %#v", failure.Facts)
	}
	if !hasFact(failure.Facts, "bootstrap_error=failed to start daemon") {
		t.Fatalf("failure.Facts missing bootstrap_error, got %#v", failure.Facts)
	}
}

func TestConnectLocalAPIWithDeps_BootstrapTimeoutFailureReported(t *testing.T) {
	t.Parallel()

	userAddr := localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/user.sock"}
	timeoutErr := errors.New("timed out waiting for localapi at " + userAddr.String())

	_, _, err := connectLocalAPIWithDeps(context.Background(), "", localAPIConnectorDeps{
		currentOperatorSID: func() (string, error) { return "sid-1", nil },
		defaultSystemAddr:  func(string) (localapi.Addr, error) { return localapi.Addr{}, errors.New("system unavailable") },
		defaultUserAddr:    func(string) (localapi.Addr, error) { return userAddr, nil },
		bundleProbeAddr:    noBundleLocalAPIProbeAddr,
		probe: func(context.Context, localapi.Addr) (*localapi.Client, error) {
			return nil, errors.New("dial failed")
		},
		bootstrap: func(addr localapi.Addr) error {
			if addr != userAddr {
				t.Fatalf("bootstrap(addr=%v), want %v", addr, userAddr)
			}
			return timeoutErr
		},
	})

	failure := requireLocalAPIConnectionFailure(t, err)
	if failure.ReasonCode != poc.ReasonCodeDaemonNotRunning {
		t.Fatalf("failure.ReasonCode = %q, want %q", failure.ReasonCode, poc.ReasonCodeDaemonNotRunning)
	}
	if !hasFact(failure.Facts, "bootstrap_error="+timeoutErr.Error()) {
		t.Fatalf("failure.Facts missing timeout fact, got %#v", failure.Facts)
	}
}

func TestConnectLocalAPIWithDeps_PermissionDeniedSkipsBootstrap(t *testing.T) {
	t.Parallel()

	userAddr := localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/user.sock"}

	bootstrapped := false
	_, _, err := connectLocalAPIWithDeps(context.Background(), "", localAPIConnectorDeps{
		currentOperatorSID: func() (string, error) { return "sid-1", nil },
		defaultSystemAddr:  func(string) (localapi.Addr, error) { return localapi.Addr{}, errors.New("system unavailable") },
		defaultUserAddr:    func(string) (localapi.Addr, error) { return userAddr, nil },
		bundleProbeAddr:    noBundleLocalAPIProbeAddr,
		probe: func(_ context.Context, addr localapi.Addr) (*localapi.Client, error) {
			if addr != userAddr {
				t.Fatalf("probe(addr=%v), want %v", addr, userAddr)
			}
			return nil, os.ErrPermission
		},
		bootstrap: func(localapi.Addr) error {
			bootstrapped = true
			return nil
		},
	})

	failure := requireLocalAPIConnectionFailure(t, err)
	if failure.ReasonCode != poc.ReasonCodeForbidden {
		t.Fatalf("failure.ReasonCode = %q, want %q", failure.ReasonCode, poc.ReasonCodeForbidden)
	}
	if bootstrapped {
		t.Fatalf("bootstrap() called, want not called")
	}
}

func TestConnectLocalAPIWithDeps_OverrideBypassesBundleProbe(t *testing.T) {
	t.Parallel()

	overrideAddr := localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/override.sock"}
	wantClient := &localapi.Client{}

	bundleCalled := false
	client, gotAddr, err := connectLocalAPIWithDeps(context.Background(), overrideAddr.String(), localAPIConnectorDeps{
		bundleProbeAddr: func() (localapi.Addr, bool) {
			bundleCalled = true
			return localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/bundle.sock"}, true
		},
		probe: func(_ context.Context, addr localapi.Addr) (*localapi.Client, error) {
			if addr != overrideAddr {
				t.Fatalf("probe(addr=%v), want %v", addr, overrideAddr)
			}
			return wantClient, nil
		},
	})
	if err != nil {
		t.Fatalf("connectLocalAPIWithDeps() error = %v, want nil", err)
	}
	if client != wantClient {
		t.Fatalf("connectLocalAPIWithDeps() client = %p, want %p", client, wantClient)
	}
	if gotAddr != overrideAddr {
		t.Fatalf("connectLocalAPIWithDeps() addr = %v, want %v", gotAddr, overrideAddr)
	}
	if bundleCalled {
		t.Fatalf("bundleProbeAddr() called, want not called")
	}
}

func TestConnectLocalAPIWithDeps_PrefersBundleProbeAddr(t *testing.T) {
	t.Parallel()

	bundleAddr := localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/bundle.sock"}
	wantClient := &localapi.Client{}

	client, gotAddr, err := connectLocalAPIWithDeps(context.Background(), "", localAPIConnectorDeps{
		currentOperatorSID: func() (string, error) {
			t.Fatal("currentOperatorSID() called, want not called")
			return "", nil
		},
		bundleProbeAddr: func() (localapi.Addr, bool) {
			return bundleAddr, true
		},
		probe: func(_ context.Context, addr localapi.Addr) (*localapi.Client, error) {
			if addr != bundleAddr {
				t.Fatalf("probe(addr=%v), want %v", addr, bundleAddr)
			}
			return wantClient, nil
		},
		bootstrap: func(localapi.Addr) error {
			t.Fatal("bootstrap() called, want not called")
			return nil
		},
	})
	if err != nil {
		t.Fatalf("connectLocalAPIWithDeps() error = %v, want nil", err)
	}
	if client != wantClient {
		t.Fatalf("connectLocalAPIWithDeps() client = %p, want %p", client, wantClient)
	}
	if gotAddr != bundleAddr {
		t.Fatalf("connectLocalAPIWithDeps() addr = %v, want %v", gotAddr, bundleAddr)
	}
}

func TestConnectLocalAPIWithDeps_BundleProbeFallsBackToPrimary(t *testing.T) {
	t.Parallel()

	bundleAddr := localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/bundle.sock"}
	userAddr := localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/user.sock"}
	wantClient := &localapi.Client{}

	bootstrapped := false
	client, gotAddr, err := connectLocalAPIWithDeps(context.Background(), "", localAPIConnectorDeps{
		currentOperatorSID: func() (string, error) { return "sid-1", nil },
		defaultSystemAddr:  func(string) (localapi.Addr, error) { return localapi.Addr{}, errors.New("system unavailable") },
		defaultUserAddr:    func(string) (localapi.Addr, error) { return userAddr, nil },
		bundleProbeAddr: func() (localapi.Addr, bool) {
			return bundleAddr, true
		},
		probe: func(_ context.Context, addr localapi.Addr) (*localapi.Client, error) {
			switch addr {
			case bundleAddr:
				return nil, errors.New("dial failed")
			case userAddr:
				return wantClient, nil
			default:
				t.Fatalf("probe(addr=%v), want bundle or primary addr", addr)
				return nil, nil
			}
		},
		bootstrap: func(localapi.Addr) error {
			bootstrapped = true
			return nil
		},
	})
	if err != nil {
		t.Fatalf("connectLocalAPIWithDeps() error = %v, want nil", err)
	}
	if client != wantClient {
		t.Fatalf("connectLocalAPIWithDeps() client = %p, want %p", client, wantClient)
	}
	if gotAddr != userAddr {
		t.Fatalf("connectLocalAPIWithDeps() addr = %v, want %v", gotAddr, userAddr)
	}
	if bootstrapped {
		t.Fatalf("bootstrap() called, want not called")
	}
}

func TestConnectLocalAPIWithDeps_BundleProbeFailureStillBootstrapsPrimary(t *testing.T) {
	t.Parallel()

	bundleAddr := localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/bundle.sock"}
	userAddr := localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/user.sock"}
	wantClient := &localapi.Client{}

	bootstrapped := false
	client, gotAddr, err := connectLocalAPIWithDeps(context.Background(), "", localAPIConnectorDeps{
		currentOperatorSID: func() (string, error) { return "sid-1", nil },
		defaultSystemAddr:  func(string) (localapi.Addr, error) { return localapi.Addr{}, errors.New("system unavailable") },
		defaultUserAddr:    func(string) (localapi.Addr, error) { return userAddr, nil },
		bundleProbeAddr: func() (localapi.Addr, bool) {
			return bundleAddr, true
		},
		probe: func(_ context.Context, addr localapi.Addr) (*localapi.Client, error) {
			switch addr {
			case bundleAddr:
				return nil, errors.New("dial failed")
			case userAddr:
				if bootstrapped {
					return wantClient, nil
				}
				return nil, errors.New("dial failed")
			default:
				t.Fatalf("probe(addr=%v), want bundle or primary addr", addr)
				return nil, nil
			}
		},
		bootstrap: func(addr localapi.Addr) error {
			if addr != userAddr {
				t.Fatalf("bootstrap(addr=%v), want %v", addr, userAddr)
			}
			bootstrapped = true
			return nil
		},
	})
	if err != nil {
		t.Fatalf("connectLocalAPIWithDeps() error = %v, want nil", err)
	}
	if client != wantClient {
		t.Fatalf("connectLocalAPIWithDeps() client = %p, want %p", client, wantClient)
	}
	if gotAddr != userAddr {
		t.Fatalf("connectLocalAPIWithDeps() addr = %v, want %v", gotAddr, userAddr)
	}
	if !bootstrapped {
		t.Fatalf("bootstrap() not called, want called")
	}
}

func TestConnectLocalAPIWithDeps_BundleProbePermissionDeniedStopsResolution(t *testing.T) {
	t.Parallel()

	bundleAddr := localapi.Addr{Transport: localapi.TransportUnix, Path: "/tmp/bundle.sock"}

	_, _, err := connectLocalAPIWithDeps(context.Background(), "", localAPIConnectorDeps{
		currentOperatorSID: func() (string, error) {
			t.Fatal("currentOperatorSID() called, want not called")
			return "", nil
		},
		bundleProbeAddr: func() (localapi.Addr, bool) {
			return bundleAddr, true
		},
		probe: func(_ context.Context, addr localapi.Addr) (*localapi.Client, error) {
			if addr != bundleAddr {
				t.Fatalf("probe(addr=%v), want %v", addr, bundleAddr)
			}
			return nil, os.ErrPermission
		},
		bootstrap: func(localapi.Addr) error {
			t.Fatal("bootstrap() called, want not called")
			return nil
		},
	})

	failure := requireLocalAPIConnectionFailure(t, err)
	if failure.ReasonCode != poc.ReasonCodeForbidden {
		t.Fatalf("failure.ReasonCode = %q, want %q", failure.ReasonCode, poc.ReasonCodeForbidden)
	}
	if !hasFact(failure.Facts, "endpoint=bundle") {
		t.Fatalf("failure.Facts missing bundle endpoint, got %#v", failure.Facts)
	}
	if !hasFact(failure.Facts, "addr="+bundleAddr.String()) {
		t.Fatalf("failure.Facts missing bundle addr, got %#v", failure.Facts)
	}
}

func noBundleLocalAPIProbeAddr() (localapi.Addr, bool) {
	return localapi.Addr{}, false
}

func requireLocalAPIConnectionFailure(t *testing.T, err error) failureOutput {
	t.Helper()

	var connErr *localAPIConnectionError
	if !errors.As(err, &connErr) {
		t.Fatalf("error = %T, want *localAPIConnectionError", err)
	}
	return connErr.Failure
}

func hasFact(facts []poc.Fact, want string) bool {
	for _, fact := range facts {
		if fact.Message == want {
			return true
		}
	}
	return false
}
