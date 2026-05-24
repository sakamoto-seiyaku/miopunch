package task

import (
	"regexp"

	"github.com/miopunch/miopunch/internal/logutil"
	"github.com/miopunch/miopunch/internal/poc"
)

var taskLogRedactRules = []struct {
	re   *regexp.Regexp
	repl string
}{
	{re: regexp.MustCompile(`(?m)(invite_code=)[^\s]+`), repl: `${1}<redacted>`},
	{re: regexp.MustCompile(`(?m)(secret_key=)[^\s]+`), repl: `${1}<redacted>`},
	{re: regexp.MustCompile(`(?m)(net_secret_b64=)[^\s]+`), repl: `${1}<redacted>`},
	{re: regexp.MustCompile(`(?m)(invite_secret_b64=)[^\s]+`), repl: `${1}<redacted>`},
}

func logTaskStage(taskID string, kind string, stage poc.Stage, message string) {
	logutil.Infof(
		"task stage: task_id=%s kind=%s stage=%s message=%s",
		taskID,
		kind,
		stage,
		redactTaskLogString(message),
	)
}

func logTaskFact(taskID string, kind string, fact poc.Fact) {
	logutil.Infof(
		"task fact: task_id=%s kind=%s term_id=%s message=%s",
		taskID,
		kind,
		fact.TermID,
		redactTaskLogString(fact.Message),
	)
}

func logTaskSuggestion(taskID string, kind string, suggestion poc.Suggestion) {
	logutil.Infof(
		"task suggestion: task_id=%s kind=%s message=%s",
		taskID,
		kind,
		redactTaskLogString(suggestion.Message),
	)
}

func logTaskDone(taskID string, kind string, reasonCode poc.ReasonCode, exitCode poc.ExitCode) {
	logutil.Infof(
		"task done: task_id=%s kind=%s reason_code=%s exit_code=%d",
		taskID,
		kind,
		reasonCode,
		exitCode,
	)
}

func redactTaskLogString(s string) string {
	for _, rule := range taskLogRedactRules {
		s = rule.re.ReplaceAllString(s, rule.repl)
	}
	return s
}
