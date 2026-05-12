//go:build windows

package shelltarget

import (
	"errors"
	"fmt"
	"os"
	"strings"
	"sync"
	"unsafe"

	"github.com/miopunch/miopunch/internal/logutil"
	"golang.org/x/sys/windows"
)

type conPTY struct {
	pcon windows.Handle

	in  *os.File
	out *os.File

	proc    windows.Handle
	thread  windows.Handle
	procID  uint32
	cmdline string

	readStartOnce  sync.Once
	readReturnOnce sync.Once
	writeOnce      sync.Once
	resizeOnce     sync.Once
	closeOnce      sync.Once
}

// Cursor inheritance requires a parent console host that can answer cursor queries.
const conPTYCreatePseudoConsoleFlags uint32 = 0

var procUpdateProcThreadAttribute = windows.NewLazySystemDLL("kernel32.dll").NewProc("UpdateProcThreadAttribute")

func startConPTY(application string, args []string, cols, rows int) (*conPTY, error) {
	if cols <= 0 {
		cols = 80
	}
	if rows <= 0 {
		rows = 24
	}

	var inRead, inWrite windows.Handle
	if err := windows.CreatePipe(&inRead, &inWrite, nil, 0); err != nil {
		return nil, err
	}
	var outRead, outWrite windows.Handle
	if err := windows.CreatePipe(&outRead, &outWrite, nil, 0); err != nil {
		_ = windows.CloseHandle(inRead)
		_ = windows.CloseHandle(inWrite)
		return nil, err
	}

	var pcon windows.Handle
	logutil.Infof(
		"conpty create start: application=%s size=%dx%d flags=%d",
		application,
		cols,
		rows,
		conPTYCreatePseudoConsoleFlags,
	)
	if err := windows.CreatePseudoConsole(
		windows.Coord{X: int16(cols), Y: int16(rows)},
		inRead,
		outWrite,
		conPTYCreatePseudoConsoleFlags,
		&pcon,
	); err != nil {
		_ = windows.CloseHandle(inRead)
		_ = windows.CloseHandle(inWrite)
		_ = windows.CloseHandle(outRead)
		_ = windows.CloseHandle(outWrite)
		return nil, err
	}

	attrList, err := windows.NewProcThreadAttributeList(1)
	if err != nil {
		windows.ClosePseudoConsole(pcon)
		_ = windows.CloseHandle(inRead)
		_ = windows.CloseHandle(inWrite)
		_ = windows.CloseHandle(outRead)
		_ = windows.CloseHandle(outWrite)
		return nil, err
	}
	defer attrList.Delete()

	if err := updatePseudoConsoleAttribute(attrList, pcon); err != nil {
		windows.ClosePseudoConsole(pcon)
		_ = windows.CloseHandle(inRead)
		_ = windows.CloseHandle(inWrite)
		_ = windows.CloseHandle(outRead)
		_ = windows.CloseHandle(outWrite)
		return nil, err
	}

	cmdline := buildCommandLine(application, args)
	cmdline16, err := windows.UTF16FromString(cmdline)
	if err != nil {
		windows.ClosePseudoConsole(pcon)
		_ = windows.CloseHandle(inRead)
		_ = windows.CloseHandle(inWrite)
		_ = windows.CloseHandle(outRead)
		_ = windows.CloseHandle(outWrite)
		return nil, err
	}
	si := windows.StartupInfoEx{}
	si.Cb = uint32(unsafe.Sizeof(si))
	si.ProcThreadAttributeList = attrList.List()
	// Empty std handles prevent console parents from bypassing the pseudoconsole pipes.
	si.Flags = windows.STARTF_USESTDHANDLES

	pi := windows.ProcessInformation{}
	currentDir16, err := conPTYCurrentDir()
	if err != nil {
		windows.ClosePseudoConsole(pcon)
		_ = windows.CloseHandle(inRead)
		_ = windows.CloseHandle(inWrite)
		_ = windows.CloseHandle(outRead)
		_ = windows.CloseHandle(outWrite)
		return nil, err
	}

	flags := uint32(windows.EXTENDED_STARTUPINFO_PRESENT | windows.CREATE_UNICODE_ENVIRONMENT)
	if err := windows.CreateProcess(nil, &cmdline16[0], nil, nil, false, flags, nil, currentDir16, &si.StartupInfo, &pi); err != nil {
		windows.ClosePseudoConsole(pcon)
		_ = windows.CloseHandle(inRead)
		_ = windows.CloseHandle(inWrite)
		_ = windows.CloseHandle(outRead)
		_ = windows.CloseHandle(outWrite)
		return nil, err
	}
	_ = windows.CloseHandle(inRead)
	_ = windows.CloseHandle(outWrite)
	logutil.Infof(
		"conpty process started: pid=%d application=%s create_flags=%d conpty_flags=%d command_line=%q",
		pi.ProcessId,
		application,
		flags,
		conPTYCreatePseudoConsoleFlags,
		cmdline,
	)

	return &conPTY{
		pcon:    pcon,
		in:      os.NewFile(uintptr(inWrite), "conpty-in"),
		out:     os.NewFile(uintptr(outRead), "conpty-out"),
		proc:    pi.Process,
		thread:  pi.Thread,
		procID:  pi.ProcessId,
		cmdline: cmdline,
	}, nil
}

func conPTYCurrentDir() (*uint16, error) {
	systemRoot := strings.TrimSpace(os.Getenv("SystemRoot"))
	if systemRoot == "" {
		systemRoot = `C:\Windows`
	}
	return windows.UTF16PtrFromString(systemRoot + `\System32`)
}

func updatePseudoConsoleAttribute(attrList *windows.ProcThreadAttributeListContainer, pcon windows.Handle) error {
	if attrList == nil || attrList.List() == nil {
		return errors.New("nil process thread attribute list")
	}
	r1, _, err := procUpdateProcThreadAttribute.Call(
		uintptr(unsafe.Pointer(attrList.List())),
		0,
		windows.PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
		uintptr(pcon),
		unsafe.Sizeof(pcon),
		0,
		0,
	)
	if r1 != 0 {
		return nil
	}
	if err != windows.ERROR_SUCCESS {
		return err
	}
	return errors.New("UpdateProcThreadAttribute failed")
}

func buildCommandLine(application string, args []string) string {
	parts := make([]string, 0, 1+len(args))
	if strings.TrimSpace(application) != "" {
		parts = append(parts, windows.EscapeArg(application))
	}
	for _, a := range args {
		parts = append(parts, windows.EscapeArg(a))
	}
	return strings.Join(parts, " ")
}

func (p *conPTY) Read(b []byte) (int, error) {
	if p == nil || p.out == nil {
		return 0, os.ErrClosed
	}
	p.readStartOnce.Do(func() {
		logutil.Infof("conpty read wait start: pid=%d command_line=%q", p.procID, p.cmdline)
	})
	n, err := p.out.Read(b)
	p.readReturnOnce.Do(func() {
		logutil.Infof("conpty first read returned: pid=%d bytes=%d err=%v", p.procID, n, err)
	})
	return n, err
}

func (p *conPTY) Write(b []byte) (int, error) {
	if p == nil || p.in == nil {
		return 0, os.ErrClosed
	}
	n, err := p.in.Write(b)
	p.writeOnce.Do(func() {
		logutil.Infof("conpty first write returned: pid=%d bytes=%d requested=%d err=%v", p.procID, n, len(b), err)
	})
	return n, err
}

func (p *conPTY) Resize(cols, rows int) error {
	if p == nil || p.pcon == 0 {
		return nil
	}
	if cols <= 0 || rows <= 0 {
		return nil
	}
	logFirstResize := false
	p.resizeOnce.Do(func() {
		logFirstResize = true
		logutil.Infof("conpty resize start: pid=%d size=%dx%d", p.procID, cols, rows)
	})
	err := windows.ResizePseudoConsole(p.pcon, windows.Coord{X: int16(cols), Y: int16(rows)})
	if logFirstResize {
		logutil.Infof("conpty resize done: pid=%d size=%dx%d err=%v", p.procID, cols, rows, err)
	}
	return err
}

func (p *conPTY) Wait() error {
	if p == nil || p.proc == 0 {
		return nil
	}
	_, err := windows.WaitForSingleObject(p.proc, windows.INFINITE)
	if err != nil {
		return err
	}
	var exitCode uint32
	if err := windows.GetExitCodeProcess(p.proc, &exitCode); err != nil {
		return err
	}
	if exitCode == 0 {
		return nil
	}
	return fmt.Errorf("process exited: %d", exitCode)
}

func (p *conPTY) Close() error {
	if p == nil {
		return nil
	}
	p.closeOnce.Do(func() {
		if p.proc != 0 {
			_ = windows.TerminateProcess(p.proc, 1)
		}
		if p.thread != 0 {
			_ = windows.CloseHandle(p.thread)
		}
		if p.proc != 0 {
			_ = windows.CloseHandle(p.proc)
		}

		if p.pcon != 0 {
			windows.ClosePseudoConsole(p.pcon)
		}
		if p.in != nil {
			_ = p.in.Close()
		}
		if p.out != nil {
			_ = p.out.Close()
		}
	})
	return nil
}
