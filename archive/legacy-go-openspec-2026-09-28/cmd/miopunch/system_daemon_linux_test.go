//go:build linux

package main

import (
	"errors"
	"testing"
)

func TestPickLinuxOperatorUser(t *testing.T) {
	t.Parallel()

	testCases := []struct {
		name        string
		sudoUser    string
		doasUser    string
		pkexecUID   string
		currentUser string
		lookupByID  func(string) (string, error)
		wantUser    string
		wantErr     bool
	}{
		{
			name:        "sudo user wins",
			sudoUser:    "alice",
			doasUser:    "bob",
			currentUser: "root",
			lookupByID: func(string) (string, error) {
				return "ignored", nil
			},
			wantUser: "alice",
		},
		{
			name:        "doas user fallback",
			doasUser:    "bob",
			currentUser: "root",
			lookupByID: func(string) (string, error) {
				return "ignored", nil
			},
			wantUser: "bob",
		},
		{
			name:      "pkexec uid lookup",
			pkexecUID: "1000",
			lookupByID: func(uid string) (string, error) {
				if uid != "1000" {
					t.Fatalf("lookupByID(%q) called, want %q", uid, "1000")
				}
				return "carol", nil
			},
			wantUser: "carol",
		},
		{
			name:        "current user fallback",
			currentUser: "root",
			lookupByID: func(string) (string, error) {
				return "", nil
			},
			wantUser: "root",
		},
		{
			name:      "pkexec lookup error",
			pkexecUID: "1001",
			lookupByID: func(string) (string, error) {
				return "", errors.New("boom")
			},
			wantErr: true,
		},
		{
			name: "empty current user errors",
			lookupByID: func(string) (string, error) {
				return "", nil
			},
			wantErr: true,
		},
	}

	for _, testCase := range testCases {
		testCase := testCase
		t.Run(testCase.name, func(t *testing.T) {
			t.Parallel()

			gotUser, err := pickLinuxOperatorUser(
				testCase.sudoUser,
				testCase.doasUser,
				testCase.pkexecUID,
				testCase.lookupByID,
				testCase.currentUser,
			)
			if gotErr := err != nil; gotErr != testCase.wantErr {
				t.Fatalf("pickLinuxOperatorUser() error = %v, want error=%t", err, testCase.wantErr)
			}
			if testCase.wantErr {
				return
			}
			if gotUser != testCase.wantUser {
				t.Fatalf("pickLinuxOperatorUser() = %q, want %q", gotUser, testCase.wantUser)
			}
		})
	}
}
