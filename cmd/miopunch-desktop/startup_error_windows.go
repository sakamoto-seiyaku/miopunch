//go:build desktop && windows

package main

import (
	"fmt"
	"os"

	"golang.org/x/sys/windows"
)

func reportStartupError(err error) {
	if err == nil {
		return
	}

	msg := "miopunch-desktop failed to start.\r\n\r\n" +
		"This may be caused by missing Microsoft Edge WebView2 Runtime.\r\n\r\n" +
		"Install WebView2 Runtime (Evergreen) and retry:\r\n" +
		"https://developer.microsoft.com/microsoft-edge/webview2/\r\n\r\n" +
		"Error:\r\n" + err.Error()

	text, textErr := windows.UTF16PtrFromString(msg)
	if textErr != nil {
		msg = "miopunch-desktop failed to start.\r\n\r\nError:\r\n" + err.Error()
		text, _ = windows.UTF16PtrFromString(msg)
	}

	caption, _ := windows.UTF16PtrFromString("miopunch")
	_, _ = windows.MessageBox(0, text, caption, windows.MB_OK|windows.MB_ICONERROR)

	fmt.Fprintln(os.Stderr, msg)
}
