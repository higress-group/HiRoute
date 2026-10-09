package management

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v8/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v8/internal/runtime/executor"
	fileauth "github.com/router-for-me/CLIProxyAPI/v8/sdk/auth"
	coreauth "github.com/router-for-me/CLIProxyAPI/v8/sdk/cliproxy/auth"
)

type managedVersionTransport func(*http.Request) (*http.Response, error)

func (f managedVersionTransport) RoundTrip(r *http.Request) (*http.Response, error) { return f(r) }

// Run against source.json plus its pinned patch. No watcher or real account is used.
func TestHiRouteManagedVersionSyncBeforeWatcher(t *testing.T) {
	for _, old := range []any{nil, "0.147.0"} {
		for _, carryVersion := range []bool{false, true} {
			dir := t.TempDir()
			name := "hiroute-managed-codex.json"
			path := filepath.Join(dir, name)
			store := fileauth.NewFileTokenStore()
			store.SetBaseDir(dir)
			manager := coreauth.NewManager(store, nil, nil)
			auth := &coreauth.Auth{
				ID: name, FileName: name, Provider: "codex", Status: coreauth.StatusActive,
				Attributes: map[string]string{"path": path},
				Metadata:   map[string]any{"type": "codex", "access_token": "fixture-access", "account_id": "fixture-account", "hiroute_client_version": old},
			}
			if _, err := manager.Register(context.Background(), auth); err != nil {
				t.Fatal(err)
			}
			fresh := map[string]any{"type": "codex", "access_token": "fixture-access", "account_id": "fixture-account", "hiroute_client_version": "0.162.0"}
			bytes, _ := json.Marshal(fresh)
			if err := os.WriteFile(path, bytes, 0o600); err != nil {
				t.Fatal(err)
			}
			// The manager remains old while the file is new: PATCH wins over the watcher.
			h := NewHandlerWithoutConfigFilePath(&config.Config{AuthDir: dir}, manager)
			var seenVersion any
			hookCalls, modelCalls, models := 0, 0, 0
			h.SetPostAuthPersistHook(func(ctx context.Context, updated *coreauth.Auth) error {
				hookCalls++
				seenVersion = updated.Metadata["hiroute_client_version"]
				ctx = context.WithValue(ctx, "cliproxy.roundtripper", managedVersionTransport(func(r *http.Request) (*http.Response, error) {
					modelCalls++
					if r.URL.Query().Get("client_version") != seenVersion || r.Header.Get("Authorization") != "Bearer fixture-access" {
						t.Fatal("discovery did not consume the synchronized account version")
					}
					return &http.Response{StatusCode: 200, Header: make(http.Header), Body: io.NopCloser(strings.NewReader(`{"models":[{"slug":"fresh-model"}]}`)), Request: r}, nil
				}))
				result, err := executor.NewCodexExecutor(nil).DiscoverModels(ctx, updated, nil)
				if carryVersion && err != nil {
					t.Fatal(err)
				}
				models = len(result)
				return nil
			})
			body := map[string]any{"name": name, "prefix": "hiroute-codex-current", "request_retry": 0, "disable_cooling": true}
			if carryVersion {
				body["hiroute_client_version"] = "0.162.0"
			}
			encoded, _ := json.Marshal(body)
			rec := httptest.NewRecorder()
			ctx, _ := gin.CreateTestContext(rec)
			ctx.Request = httptest.NewRequest(http.MethodPatch, "/v0/management/auth-files/fields", strings.NewReader(string(encoded)))
			ctx.Request.Header.Set("Content-Type", "application/json")
			h.PatchAuthFileFields(ctx)
			if rec.Code != http.StatusOK || hookCalls != 1 {
				t.Fatalf("synchronous patch failed: %d / %d", rec.Code, hookCalls)
			}
			persisted, err := os.ReadFile(path)
			if err != nil {
				t.Fatal(err)
			}
			var disk map[string]any
			if err := json.Unmarshal(persisted, &disk); err != nil {
				t.Fatal(err)
			}
			want := old
			if carryVersion {
				want = "0.162.0"
			}
			if disk["hiroute_client_version"] != want || seenVersion != want {
				t.Fatal("manager, file and synchronous discovery disagree")
			}
			if carryVersion && (models != 1 || modelCalls != 1) {
				t.Fatal("fresh discovery did not execute")
			}
			if old == nil && !carryVersion && (models != 0 || modelCalls != 0) {
				t.Fatal("legacy missing version unexpectedly discovered models")
			}
			if disk["access_token"] != "fixture-access" || disk["account_id"] != "fixture-account" {
				t.Fatal("account identity or access lease changed")
			}
			if _, ok := disk["refresh_token"]; ok {
				t.Fatal("refresh token entered CPA")
			}
		}
	}
}
