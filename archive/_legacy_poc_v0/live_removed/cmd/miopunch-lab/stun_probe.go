package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/miopunch/miopunch/connectivity"
	"github.com/miopunch/miopunch/internal/netutil"
	"github.com/miopunch/miopunch/internal/stunclient"
)

type stunProbeRecord struct {
	Endpoint           string   `json:"endpoint"`
	NormalizedHostPort string   `json:"normalized_hostport,omitempty"`
	Bucket             string   `json:"bucket,omitempty"`
	UDPOkCount         int      `json:"udp_ok_count"`
	TCPOkCount         int      `json:"tcp_ok_count"`
	UDPRTTMsMin        int      `json:"udp_rtt_ms_min"`
	TCPRTTMsMin        int      `json:"tcp_rtt_ms_min"`
	UDPMappedAddrs     []string `json:"udp_mapped_addrs"`
	TCPMappedAddrs     []string `json:"tcp_mapped_addrs"`
	Decision           string   `json:"decision"`
	Errors             []string `json:"errors,omitempty"`
}

func stunProbeCmd(ctx context.Context, args []string, stdout, stderr io.Writer) int {
	fs := flag.NewFlagSet("stun probe", flag.ContinueOnError)
	fs.SetOutput(stderr)

	builtin := fs.Bool("builtin", false, "probe built-in stun endpoints")
	stunServers := fs.String("stun", "", "comma-separated stun endpoints (host:port, udp://..., tcp://...)")
	attempts := fs.Int("attempts", 3, "attempts per endpoint per protocol")
	okThreshold := fs.Int("ok-threshold", 2, "ok threshold for classification")
	timeout := fs.Duration("timeout", 2*time.Second, "per-attempt timeout")
	dialTimeout := fs.Duration("dial-timeout", 2*time.Second, "tcp dial timeout")
	concurrency := fs.Int("concurrency", 8, "concurrency across endpoints")
	builtinDNSMode := fs.String("builtin-dns-mode", "auto", "built-in dns mode: auto|on|off")
	builtinDNS := fs.String("builtin-dns", "", "comma-separated built-in dns resolvers (ip[:port]) (TCP/53)")
	outPath := fs.String("out", "", "optional output file path (writes the same JSONL as stdout)")
	logLevel := addLogLevelFlag(fs)

	if err := fs.Parse(args); err != nil {
		if err == flag.ErrHelp {
			return 0
		}
		return 2
	}
	applyLogLevel(*logLevel)

	if *builtin && strings.TrimSpace(*stunServers) != "" {
		fmt.Fprintln(stderr, "invalid config: --builtin and --stun are mutually exclusive")
		return 2
	}
	if !*builtin && strings.TrimSpace(*stunServers) == "" {
		fmt.Fprintln(stderr, "invalid config: either --builtin or --stun must be provided")
		return 2
	}
	if *attempts <= 0 {
		fmt.Fprintln(stderr, "invalid config: --attempts must be > 0")
		return 2
	}
	if *okThreshold <= 0 {
		fmt.Fprintln(stderr, "invalid config: --ok-threshold must be > 0")
		return 2
	}
	if *concurrency <= 0 {
		*concurrency = 1
	}

	resolver, err := netutil.NewDNSResolver(*builtinDNSMode, splitComma(*builtinDNS))
	if err != nil {
		fmt.Fprintln(stderr, err)
		return 2
	}

	var (
		jobs []stunProbeJob
	)
	if *builtin {
		cn, global := connectivity.BuiltinSTUNBuckets()
		jobs = append(jobs, makeJobsFromBucket("cn", cn)...)
		jobs = append(jobs, makeJobsFromBucket("global", global)...)
	} else {
		for _, ep := range splitComma(*stunServers) {
			jobs = append(jobs, stunProbeJob{Endpoint: ep})
		}
	}
	if len(jobs) == 0 {
		fmt.Fprintln(stderr, "no stun endpoints to probe")
		return 2
	}

	var outFile *os.File
	if strings.TrimSpace(*outPath) != "" {
		f, err := os.Create(*outPath)
		if err != nil {
			fmt.Fprintln(stderr, err)
			return 1
		}
		outFile = f
		defer outFile.Close()
	}

	writer := stdout
	if outFile != nil {
		writer = io.MultiWriter(stdout, outFile)
	}
	enc := json.NewEncoder(writer)

	udpConn, err := net.ListenUDP("udp4", &net.UDPAddr{IP: net.IPv4zero, Port: 0})
	if err != nil {
		fmt.Fprintln(stderr, err)
		return 1
	}
	defer udpConn.Close()

	udpClient := stunclient.NewUDPClient(udpConn)
	defer udpClient.Close()

	dialer := &net.Dialer{Timeout: *dialTimeout}

	workCh := make(chan stunProbeJob)
	resultCh := make(chan stunProbeRecord)

	var wg sync.WaitGroup
	for range *concurrency {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for job := range workCh {
				record := probeOneEndpoint(ctx, resolver, udpClient, dialer, job, *attempts, *okThreshold, *timeout)
				resultCh <- record
			}
		}()
	}

	go func() {
		for _, job := range jobs {
			workCh <- job
		}
		close(workCh)
		wg.Wait()
		close(resultCh)
	}()

	for record := range resultCh {
		if err := enc.Encode(record); err != nil {
			fmt.Fprintln(stderr, err)
			return 1
		}
	}

	return 0
}

type stunProbeJob struct {
	Endpoint string
	Bucket   string
}

func makeJobsFromBucket(bucket string, endpoints []string) []stunProbeJob {
	out := make([]stunProbeJob, 0, len(endpoints))
	for _, ep := range endpoints {
		out = append(out, stunProbeJob{Endpoint: ep, Bucket: bucket})
	}
	return out
}

func probeOneEndpoint(ctx context.Context, resolver *netutil.DNSResolver, udpClient *stunclient.UDPClient, dialer *net.Dialer, job stunProbeJob, attempts, okThreshold int, timeout time.Duration) stunProbeRecord {
	record := stunProbeRecord{
		Endpoint:       job.Endpoint,
		Bucket:         job.Bucket,
		UDPMappedAddrs: make([]string, 0),
		TCPMappedAddrs: make([]string, 0),
		Errors:         make([]string, 0),
	}

	parsed, err := stunclient.ParseEndpoint(job.Endpoint)
	if err != nil {
		record.Errors = append(record.Errors, err.Error())
		record.Decision = "remove"
		return record
	}
	record.NormalizedHostPort = parsed.HostPort

	resolved, resolveErrors := stunclient.ResolveHostPortsIP4(ctx, resolver, []string{parsed.HostPort}, 0)
	if len(resolveErrors) > 0 {
		record.Errors = append(record.Errors, resolveErrors...)
	}
	if len(resolved) == 0 {
		record.Decision = "remove"
		return record
	}

	var udpRTTMin time.Duration
	var tcpRTTMin time.Duration

	for i := 0; i < attempts; i++ {
		target := resolved[i%len(resolved)]

		udpCtx, udpCancel := context.WithTimeout(ctx, timeout)
		addrs, rtt, err := stunclient.DiscoverFromServerUDP(udpCtx, udpClient, target)
		udpCancel()
		if err == nil {
			record.UDPOkCount++
			if udpRTTMin == 0 || (rtt > 0 && rtt < udpRTTMin) {
				udpRTTMin = rtt
			}
			record.UDPMappedAddrs = append(record.UDPMappedAddrs, addrs...)
		} else {
			record.Errors = append(record.Errors, fmt.Sprintf("udp[%d] %s: %v", i+1, target, err))
		}

		tcpCtx, tcpCancel := context.WithTimeout(ctx, timeout)
		mapped, rtt, err := stunclient.RoundTripTCP(tcpCtx, dialer, target)
		tcpCancel()
		if err == nil && mapped != "" {
			record.TCPOkCount++
			if tcpRTTMin == 0 || (rtt > 0 && rtt < tcpRTTMin) {
				tcpRTTMin = rtt
			}
			record.TCPMappedAddrs = append(record.TCPMappedAddrs, mapped)
		} else if err != nil {
			record.Errors = append(record.Errors, fmt.Sprintf("tcp[%d] %s: %v", i+1, target, err))
		}
	}

	record.UDPMappedAddrs = dedupStrings(record.UDPMappedAddrs)
	record.TCPMappedAddrs = dedupStrings(record.TCPMappedAddrs)

	if udpRTTMin > 0 {
		record.UDPRTTMsMin = int(udpRTTMin.Milliseconds())
	}
	if tcpRTTMin > 0 {
		record.TCPRTTMsMin = int(tcpRTTMin.Milliseconds())
	}

	udpOK := record.UDPOkCount >= okThreshold
	tcpOK := record.TCPOkCount >= okThreshold
	switch {
	case udpOK && tcpOK:
		record.Decision = "dual"
	case udpOK:
		record.Decision = "udp_only"
	case tcpOK:
		record.Decision = "tcp_only"
	default:
		record.Decision = "remove"
	}

	if len(record.Errors) == 0 {
		record.Errors = nil
	}
	return record
}

func dedupStrings(in []string) []string {
	if len(in) <= 1 {
		return in
	}
	seen := make(map[string]struct{}, len(in))
	out := make([]string, 0, len(in))
	for _, s := range in {
		if _, ok := seen[s]; ok {
			continue
		}
		seen[s] = struct{}{}
		out = append(out, s)
	}
	return out
}
