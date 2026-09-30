#!/usr/bin/env python3
"""Export the archived Go classifier/selector as a read-only differential oracle.

Only package/import plumbing is adapted; the old algorithms execute verbatim.
No archived project-wide gates or dependencies are restored.
"""
import argparse
import pathlib
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("output", type=pathlib.Path)
    args = parser.parse_args()
    repo = pathlib.Path(__file__).resolve().parents[1]
    archive = repo / "archive/legacy-go-openspec-2026-09-28"
    with tempfile.TemporaryDirectory(prefix="miopunch62-go-reference-") as directory:
        temporary = pathlib.Path(directory)
        classify = (archive / "nat/classify.go").read_text().replace("package nat", "package main", 1)
        analysis = (archive / "internal/punchdecision/nathole_analysis.go").read_text()
        analysis = analysis.replace("package punchdecision", "package main", 1)
        analysis = analysis.replace('\n\t"github.com/samber/lo"', "")
        analysis = analysis.replace("lo.Tuple2", "Tuple2").replace("lo.T2", "T2")
        (temporary / "classify.go").write_text(classify)
        (temporary / "analysis.go").write_text(analysis)
        (temporary / "main.go").write_text('''package main
import ("encoding/json"; "os")
const (DetectRoleSender="sender"; DetectRoleReceiver="receiver"; DetectMode0=0; DetectMode1=1; DetectMode2=2; DetectMode3=3; DetectMode4=4)
type Tuple2[A any,B any] struct { A A; B B }
func T2[A any,B any](a A,b B) Tuple2[A,B] { return Tuple2[A,B]{a,b} }
type Entry struct { Mode int; Index int; C RecommandBehavior; V RecommandBehavior }
func main() {
  entries:=[]Entry{}
  for mode:=0;mode<=4;mode++ {for index,pair:=range getBehaviorByMode(mode) {entries=append(entries,Entry{mode,index,pair.A,pair.B})}}
  if err:=json.NewEncoder(os.Stdout).Encode(entries);err!=nil {panic(err)}
}
''')
        output = subprocess.check_output(["rtk", "proxy", "go", "run", "classify.go", "analysis.go", "main.go"], cwd=temporary)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(output)


if __name__ == "__main__":
    main()
