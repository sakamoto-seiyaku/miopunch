//go:build desktop && windows

package main

import "testing"

func TestTrayCallbackEventDecodesVersion4LowWord(t *testing.T) {
	tests := []struct {
		name   string
		lParam uintptr
		want   uint32
	}{
		{
			name:   "context menu with icon ID",
			lParam: uintptr(trayID<<16 | wmContextMenu),
			want:   wmContextMenu,
		},
		{
			name:   "right button with icon ID",
			lParam: uintptr(trayID<<16 | wmRButtonUp),
			want:   wmRButtonUp,
		},
		{
			name:   "select with icon ID",
			lParam: uintptr(trayID<<16 | ninSelect),
			want:   ninSelect,
		},
		{
			name:   "legacy full event",
			lParam: uintptr(wmLButtonDblClk),
			want:   wmLButtonDblClk,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := trayCallbackEvent(tt.lParam)
			if got != tt.want {
				t.Fatalf("trayCallbackEvent(%#x) = %#x, want %#x", tt.lParam, got, tt.want)
			}
		})
	}
}

func TestTrayCallbackAction(t *testing.T) {
	tests := []struct {
		name           string
		event          uint32
		notifyVersion4 bool
		want           trayAction
	}{
		{
			name:           "version 4 context menu shows menu",
			event:          wmContextMenu,
			notifyVersion4: true,
			want:           trayActionShowMenu,
		},
		{
			name:           "version 4 right button up does not duplicate menu",
			event:          wmRButtonUp,
			notifyVersion4: true,
			want:           trayActionNone,
		},
		{
			name:           "legacy right button up shows menu",
			event:          wmRButtonUp,
			notifyVersion4: false,
			want:           trayActionShowMenu,
		},
		{
			name:           "left button opens window",
			event:          wmLButtonUp,
			notifyVersion4: true,
			want:           trayActionOpen,
		},
		{
			name:           "double click opens window",
			event:          wmLButtonDblClk,
			notifyVersion4: true,
			want:           trayActionOpen,
		},
		{
			name:           "keyboard select opens window",
			event:          ninKeySelect,
			notifyVersion4: true,
			want:           trayActionOpen,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := trayCallbackAction(tt.event, tt.notifyVersion4)
			if got != tt.want {
				t.Fatalf("trayCallbackAction(%#x, %t) = %v, want %v", tt.event, tt.notifyVersion4, got, tt.want)
			}
		})
	}
}

func TestTrayCallbackPointDecodesVersion4Coordinates(t *testing.T) {
	tests := []struct {
		name   string
		wParam uintptr
		want   windowsPoint
	}{
		{
			name:   "positive coordinates",
			wParam: trayPointParam(123, 456),
			want:   windowsPoint{X: 123, Y: 456},
		},
		{
			name:   "negative coordinates",
			wParam: trayPointParam(-12, -34),
			want:   windowsPoint{X: -12, Y: -34},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := trayCallbackPoint(tt.wParam)
			if got != tt.want {
				t.Fatalf("trayCallbackPoint(%#x) = %+v, want %+v", tt.wParam, got, tt.want)
			}
		})
	}
}

func trayPointParam(x int16, y int16) uintptr {
	return uintptr(uint16(x)) | uintptr(uint16(y))<<16
}
