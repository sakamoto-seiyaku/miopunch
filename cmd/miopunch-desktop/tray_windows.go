//go:build desktop && windows

package main

import (
	"fmt"
	goruntime "runtime"
	"sync"
	"syscall"
	"time"
	"unsafe"

	"golang.org/x/sys/windows"
)

const (
	trayCallbackMessage = 0x0400 + 1
	trayShowMenuMessage = 0x0400 + 2
	trayID              = 1
	trayOpenCommand     = 1001
	trayQuitCommand     = 1002

	wmClose          = 0x0010
	wmCommand        = 0x0111
	wmContextMenu    = 0x007B
	wmDestroy        = 0x0002
	wmLButtonDblClk  = 0x0203
	wmLButtonUp      = 0x0202
	wmNull           = 0x0000
	wmRButtonUp      = 0x0205
	ninSelect        = 0x0400
	ninKeySelect     = 0x0401
	nifMessage       = 0x00000001
	nifIcon          = 0x00000002
	nifTip           = 0x00000004
	nimAdd           = 0x00000000
	nimDelete        = 0x00000002
	nimSetFocus      = 0x00000003
	nimSetVersion    = 0x00000004
	notifyVersion4   = 4
	mfString         = 0x00000000
	tpmRightButton   = 0x00000002
	tpmLeftAlign     = 0x00000000
	tpmReturnCommand = 0x00000100
	wsOverlapped     = 0x00000000
	idiApplication   = 32512
	cwUseDefault     = 0x80000000
	trayCloseTimeout = 2 * time.Second
)

var (
	user32                = windows.NewLazySystemDLL("user32.dll")
	shell32               = windows.NewLazySystemDLL("shell32.dll")
	kernel32              = windows.NewLazySystemDLL("kernel32.dll")
	procAppendMenu        = user32.NewProc("AppendMenuW")
	procCreatePopupMenu   = user32.NewProc("CreatePopupMenu")
	procCreateWindowEx    = user32.NewProc("CreateWindowExW")
	procDefWindowProc     = user32.NewProc("DefWindowProcW")
	procDestroyMenu       = user32.NewProc("DestroyMenu")
	procDestroyWindow     = user32.NewProc("DestroyWindow")
	procDispatchMessage   = user32.NewProc("DispatchMessageW")
	procGetCursorPos      = user32.NewProc("GetCursorPos")
	procGetMessage        = user32.NewProc("GetMessageW")
	procGetModuleHandle   = kernel32.NewProc("GetModuleHandleW")
	procLoadIcon          = user32.NewProc("LoadIconW")
	procPostMessage       = user32.NewProc("PostMessageW")
	procPostQuitMessage   = user32.NewProc("PostQuitMessage")
	procRegisterClassEx   = user32.NewProc("RegisterClassExW")
	procSetForeground     = user32.NewProc("SetForegroundWindow")
	procShellNotifyIcon   = shell32.NewProc("Shell_NotifyIconW")
	procTrackPopupMenu    = user32.NewProc("TrackPopupMenu")
	procTranslateMessage  = user32.NewProc("TranslateMessage")
	windowsTrayWndProcPtr = syscall.NewCallback(windowsTrayWndProc)
	windowsTrayByHWND     sync.Map
)

type windowsTray struct {
	mu             sync.Mutex
	hwnd           uintptr
	done           chan struct{}
	notifyVersion4 bool

	onOpen func()
	onQuit func()
}

func newPlatformTray() desktopTray {
	return &windowsTray{}
}

func (t *windowsTray) Show(onOpen func(), onQuit func()) error {
	t.mu.Lock()
	if t.hwnd != 0 {
		t.onOpen = onOpen
		t.onQuit = onQuit
		t.mu.Unlock()
		return nil
	}

	ready := make(chan error, 1)
	done := make(chan struct{})
	t.done = done
	t.onOpen = onOpen
	t.onQuit = onQuit
	t.mu.Unlock()

	go t.run(ready, done)

	if err := <-ready; err != nil {
		t.mu.Lock()
		if t.done == done {
			t.done = nil
		}
		t.mu.Unlock()
		return err
	}
	return nil
}

func (t *windowsTray) Close() {
	t.mu.Lock()
	hwnd := t.hwnd
	done := t.done
	t.mu.Unlock()
	if hwnd == 0 || done == nil {
		return
	}

	_, _, _ = procPostMessage.Call(hwnd, wmClose, 0, 0)
	select {
	case <-done:
	case <-time.After(trayCloseTimeout):
	}
}

func (t *windowsTray) run(ready chan<- error, done chan struct{}) {
	goruntime.LockOSThread()
	defer goruntime.UnlockOSThread()
	defer close(done)

	hwnd, err := t.createWindow()
	if err != nil {
		ready <- err
		return
	}

	t.mu.Lock()
	t.hwnd = hwnd
	t.mu.Unlock()
	windowsTrayByHWND.Store(hwnd, t)

	notifyVersion4, err := addTrayIcon(hwnd)
	if err != nil {
		windowsTrayByHWND.Delete(hwnd)
		t.mu.Lock()
		t.hwnd = 0
		t.done = nil
		t.mu.Unlock()
		_, _, _ = procDestroyWindow.Call(hwnd)
		ready <- err
		return
	}
	t.mu.Lock()
	t.notifyVersion4 = notifyVersion4
	t.mu.Unlock()
	ready <- nil

	var msg windowsMSG
	for {
		ret, _, _ := procGetMessage.Call(uintptr(unsafe.Pointer(&msg)), 0, 0, 0)
		if int32(ret) <= 0 {
			break
		}
		_, _, _ = procTranslateMessage.Call(uintptr(unsafe.Pointer(&msg)))
		_, _, _ = procDispatchMessage.Call(uintptr(unsafe.Pointer(&msg)))
	}

	_ = deleteTrayIcon(hwnd)
	windowsTrayByHWND.Delete(hwnd)
	t.mu.Lock()
	t.hwnd = 0
	t.done = nil
	t.notifyVersion4 = false
	t.mu.Unlock()
}

func (t *windowsTray) createWindow() (uintptr, error) {
	instance, _, err := procGetModuleHandle.Call(0)
	if instance == 0 {
		return 0, fmt.Errorf("get module handle: %w", err)
	}

	className, err := windows.UTF16PtrFromString("MiopunchTrayWindow")
	if err != nil {
		return 0, fmt.Errorf("encode tray window class: %w", err)
	}
	wc := windowsWndClassEx{
		CbSize:        uint32(unsafe.Sizeof(windowsWndClassEx{})),
		LpfnWndProc:   windowsTrayWndProcPtr,
		HInstance:     instance,
		LpszClassName: className,
	}
	atom, _, registerErr := procRegisterClassEx.Call(uintptr(unsafe.Pointer(&wc)))
	if atom == 0 {
		if errno, ok := registerErr.(syscall.Errno); !ok || errno != windows.ERROR_CLASS_ALREADY_EXISTS {
			return 0, fmt.Errorf("register tray window class: %w", registerErr)
		}
	}

	windowName, err := windows.UTF16PtrFromString("miopunch tray")
	if err != nil {
		return 0, fmt.Errorf("encode tray window name: %w", err)
	}
	hwnd, _, createErr := procCreateWindowEx.Call(
		0,
		uintptr(unsafe.Pointer(className)),
		uintptr(unsafe.Pointer(windowName)),
		wsOverlapped,
		cwUseDefault,
		cwUseDefault,
		0,
		0,
		0,
		0,
		instance,
		0,
	)
	if hwnd == 0 {
		return 0, fmt.Errorf("create tray window: %w", createErr)
	}
	return hwnd, nil
}

func (t *windowsTray) open() {
	t.mu.Lock()
	onOpen := t.onOpen
	t.mu.Unlock()
	if onOpen != nil {
		go onOpen()
	}
}

func (t *windowsTray) quit() {
	t.mu.Lock()
	onQuit := t.onQuit
	t.mu.Unlock()
	if onQuit != nil {
		go onQuit()
	}
}

func (t *windowsTray) usesNotifyVersion4() bool {
	t.mu.Lock()
	defer t.mu.Unlock()
	return t.notifyVersion4
}

func addTrayIcon(hwnd uintptr) (bool, error) {
	icon, _, iconErr := procLoadIcon.Call(0, idiApplication)
	if icon == 0 {
		return false, fmt.Errorf("load tray icon: %w", iconErr)
	}

	nid := newNotifyIconData(hwnd, icon)
	ok, _, err := procShellNotifyIcon.Call(nimAdd, uintptr(unsafe.Pointer(&nid)))
	if ok == 0 {
		return false, fmt.Errorf("add tray icon: %w", err)
	}
	nid.UVersion = notifyVersion4
	versionOK, _, _ := procShellNotifyIcon.Call(nimSetVersion, uintptr(unsafe.Pointer(&nid)))
	return versionOK != 0, nil
}

func deleteTrayIcon(hwnd uintptr) error {
	nid := notifyIconData{
		CbSize: uint32(unsafe.Sizeof(notifyIconData{})),
		HWnd:   hwnd,
		UID:    trayID,
	}
	ok, _, err := procShellNotifyIcon.Call(nimDelete, uintptr(unsafe.Pointer(&nid)))
	if ok == 0 {
		return fmt.Errorf("delete tray icon: %w", err)
	}
	return nil
}

func newNotifyIconData(hwnd uintptr, icon uintptr) notifyIconData {
	nid := notifyIconData{
		CbSize:           uint32(unsafe.Sizeof(notifyIconData{})),
		HWnd:             hwnd,
		UID:              trayID,
		UFlags:           nifMessage | nifIcon | nifTip,
		UCallbackMessage: trayCallbackMessage,
		HIcon:            icon,
	}
	copyUTF16(nid.SzTip[:], "miopunch running")
	return nid
}

func windowsTrayWndProc(hwnd uintptr, msg uint32, wParam uintptr, lParam uintptr) uintptr {
	value, ok := windowsTrayByHWND.Load(hwnd)
	if !ok {
		ret, _, _ := procDefWindowProc.Call(hwnd, uintptr(msg), wParam, lParam)
		return ret
	}
	tray := value.(*windowsTray)

	switch msg {
	case trayCallbackMessage:
		version4 := tray.usesNotifyVersion4()
		switch trayCallbackAction(trayCallbackEvent(lParam), version4) {
		case trayActionOpen:
			tray.open()
			return 0
		case trayActionShowMenu:
			hasPoint := uintptr(0)
			if version4 {
				hasPoint = 1
			}
			_, _, _ = procPostMessage.Call(hwnd, trayShowMenuMessage, wParam, hasPoint)
			return 0
		}
	case trayShowMenuMessage:
		tray.showMenu(hwnd, trayCallbackPoint(wParam), lParam != 0)
		return 0
	case wmCommand:
		if tray.handleMenuCommand(uint32(wParam & 0xffff)) {
			return 0
		}
	case wmClose:
		_, _, _ = procDestroyWindow.Call(hwnd)
		return 0
	case wmDestroy:
		_, _, _ = procPostQuitMessage.Call(0)
		return 0
	}

	ret, _, _ := procDefWindowProc.Call(hwnd, uintptr(msg), wParam, lParam)
	return ret
}

func trayCallbackEvent(lParam uintptr) uint32 {
	return uint32(lParam & 0xffff)
}

type trayAction uint8

const (
	trayActionNone trayAction = iota
	trayActionOpen
	trayActionShowMenu
)

func trayCallbackAction(event uint32, notifyVersion4 bool) trayAction {
	switch event {
	case wmLButtonUp, wmLButtonDblClk, ninSelect, ninKeySelect:
		return trayActionOpen
	case wmRButtonUp:
		if !notifyVersion4 {
			return trayActionShowMenu
		}
		return trayActionNone
	case wmContextMenu:
		return trayActionShowMenu
	default:
		return trayActionNone
	}
}

func trayCallbackPoint(wParam uintptr) windowsPoint {
	return windowsPoint{
		X: int32(int16(wParam & 0xffff)),
		Y: int32(int16((wParam >> 16) & 0xffff)),
	}
}

func (t *windowsTray) showMenu(hwnd uintptr, p windowsPoint, hasPoint bool) {
	menu, _, _ := procCreatePopupMenu.Call()
	if menu == 0 {
		return
	}
	defer procDestroyMenu.Call(menu)

	openText, _ := windows.UTF16PtrFromString("Open miopunch")
	quitText, _ := windows.UTF16PtrFromString("Quit miopunch")
	_, _, _ = procAppendMenu.Call(menu, mfString, trayOpenCommand, uintptr(unsafe.Pointer(openText)))
	_, _, _ = procAppendMenu.Call(menu, mfString, trayQuitCommand, uintptr(unsafe.Pointer(quitText)))

	if !hasPoint {
		_, _, _ = procGetCursorPos.Call(uintptr(unsafe.Pointer(&p)))
	}
	_, _, _ = procSetForeground.Call(hwnd)
	command, _, _ := procTrackPopupMenu.Call(menu, tpmRightButton|tpmLeftAlign|tpmReturnCommand, uintptr(p.X), uintptr(p.Y), 0, hwnd, 0)
	_, _, _ = procPostMessage.Call(hwnd, wmNull, 0, 0)
	_ = setTrayFocus(hwnd)
	t.handleMenuCommand(uint32(command))
}

func (t *windowsTray) handleMenuCommand(command uint32) bool {
	switch command {
	case trayOpenCommand:
		t.open()
		return true
	case trayQuitCommand:
		t.quit()
		return true
	default:
		return false
	}
}

func setTrayFocus(hwnd uintptr) error {
	nid := notifyIconData{
		CbSize: uint32(unsafe.Sizeof(notifyIconData{})),
		HWnd:   hwnd,
		UID:    trayID,
	}
	ok, _, err := procShellNotifyIcon.Call(nimSetFocus, uintptr(unsafe.Pointer(&nid)))
	if ok == 0 {
		return fmt.Errorf("set tray focus: %w", err)
	}
	return nil
}

func copyUTF16(dst []uint16, value string) {
	encoded := windows.StringToUTF16(value)
	if len(encoded) > len(dst) {
		encoded = encoded[:len(dst)]
		encoded[len(encoded)-1] = 0
	}
	copy(dst, encoded)
}

type windowsWndClassEx struct {
	CbSize        uint32
	Style         uint32
	LpfnWndProc   uintptr
	CbClsExtra    int32
	CbWndExtra    int32
	HInstance     uintptr
	HIcon         uintptr
	HCursor       uintptr
	HbrBackground uintptr
	LpszMenuName  *uint16
	LpszClassName *uint16
	HIconSm       uintptr
}

type windowsPoint struct {
	X int32
	Y int32
}

type windowsMSG struct {
	HWnd    uintptr
	Message uint32
	WParam  uintptr
	LParam  uintptr
	Time    uint32
	Pt      windowsPoint
}

type notifyIconData struct {
	CbSize           uint32
	HWnd             uintptr
	UID              uint32
	UFlags           uint32
	UCallbackMessage uint32
	HIcon            uintptr
	SzTip            [128]uint16
	DwState          uint32
	DwStateMask      uint32
	SzInfo           [256]uint16
	UVersion         uint32
	SzInfoTitle      [64]uint16
	DwInfoFlags      uint32
	GuidItem         windows.GUID
	HBalloonIcon     uintptr
}
