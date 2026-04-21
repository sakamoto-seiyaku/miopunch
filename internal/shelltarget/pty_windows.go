//go:build windows

package shelltarget

import (
	"fmt"
	"os"
	"strings"
	"sync"
	"unsafe"

	"golang.org/x/sys/windows"
)

type conPTY struct {
	pcon windows.Handle

	in  *os.File
	out *os.File

	proc   windows.Handle
	thread windows.Handle

	closeOnce sync.Once
}

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
	if err := windows.CreatePseudoConsole(windows.Coord{X: int16(cols), Y: int16(rows)}, inRead, outWrite, windows.PSEUDOCONSOLE_INHERIT_CURSOR, &pcon); err != nil {
		_ = windows.CloseHandle(inRead)
		_ = windows.CloseHandle(inWrite)
		_ = windows.CloseHandle(outRead)
		_ = windows.CloseHandle(outWrite)
		return nil, err
	}
	_ = windows.CloseHandle(inRead)
	_ = windows.CloseHandle(outWrite)

	attrList, err := windows.NewProcThreadAttributeList(1)
	if err != nil {
		windows.ClosePseudoConsole(pcon)
		_ = windows.CloseHandle(inWrite)
		_ = windows.CloseHandle(outRead)
		return nil, err
	}
	defer attrList.Delete()

	if err := attrList.Update(windows.PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, unsafe.Pointer(&pcon), unsafe.Sizeof(pcon)); err != nil {
		windows.ClosePseudoConsole(pcon)
		_ = windows.CloseHandle(inWrite)
		_ = windows.CloseHandle(outRead)
		return nil, err
	}

	cmdline := buildCommandLine(application, args)
	cmdline16, err := windows.UTF16FromString(cmdline)
	if err != nil {
		windows.ClosePseudoConsole(pcon)
		_ = windows.CloseHandle(inWrite)
		_ = windows.CloseHandle(outRead)
		return nil, err
	}

	si := windows.StartupInfoEx{}
	si.Cb = uint32(unsafe.Sizeof(si))
	si.ProcThreadAttributeList = attrList.List()

	pi := windows.ProcessInformation{}
	flags := uint32(windows.EXTENDED_STARTUPINFO_PRESENT | windows.CREATE_UNICODE_ENVIRONMENT | windows.CREATE_NO_WINDOW)
	if err := windows.CreateProcess(nil, &cmdline16[0], nil, nil, false, flags, nil, nil, &si.StartupInfo, &pi); err != nil {
		windows.ClosePseudoConsole(pcon)
		_ = windows.CloseHandle(inWrite)
		_ = windows.CloseHandle(outRead)
		return nil, err
	}

	return &conPTY{
		pcon:   pcon,
		in:     os.NewFile(uintptr(inWrite), "conpty-in"),
		out:    os.NewFile(uintptr(outRead), "conpty-out"),
		proc:   pi.Process,
		thread: pi.Thread,
	}, nil
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
	return p.out.Read(b)
}

func (p *conPTY) Write(b []byte) (int, error) {
	if p == nil || p.in == nil {
		return 0, os.ErrClosed
	}
	return p.in.Write(b)
}

func (p *conPTY) Resize(cols, rows int) error {
	if p == nil || p.pcon == 0 {
		return nil
	}
	if cols <= 0 || rows <= 0 {
		return nil
	}
	return windows.ResizePseudoConsole(p.pcon, windows.Coord{X: int16(cols), Y: int16(rows)})
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
