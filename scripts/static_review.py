#!/usr/bin/env python3
"""Optional lexical/SQL review. Requires Pygments; this is not a Rust compiler."""
from pathlib import Path
from pygments import lex
from pygments.lexers import RustLexer
from pygments.token import Comment, String, Error as LexerError
import json,re,sqlite3
p=Path(__file__).resolve().parents[1]
issues=[];codes={};rustfiles=list(p.rglob('*.rs'))
for f in rustfiles:
    tokens=list(lex(f.read_text(),RustLexer()))
    code=''.join((' '*(len(v)-v.count('\n'))+'\n'*v.count('\n')) if (t in Comment or t in String) else v for t,v in tokens)
    codes[f]=code
    stack=[]
    for i,c in enumerate(code):
        if c in '([{': stack.append(c)
        elif c in ')]}':
            if not stack or '([{'.index(stack.pop())!=')]}'.index(c):issues.append([str(f.relative_to(p)),'delimiter mismatch',i]);break
    if stack: issues.append([str(f.relative_to(p)),'unclosed delimiter',str(stack[-10:])])
# Compile actual static SQL strings without executing mutations.
db=sqlite3.connect(':memory:');db.executescript((p/'migrations/0001_authority.sql').read_text());count=0;sql_errors=[]
for f in rustfiles:
    if '/acquisition/code/' in str(f):continue
    for match in re.finditer(r'"(?:\\.|[^"\\])*"',f.read_text()):
        try:q=json.loads(match.group())
        except Exception:continue
        if not re.match(r'^(SELECT|INSERT|UPDATE|DELETE|PRAGMA)\s',q):continue
        if ';' in q.strip().rstrip(';'):continue
        maxp=max([int(x) for x in re.findall(r'\?(\d+)',q)] or [0])
        try:db.execute('EXPLAIN '+q,[None]*maxp);count+=1
        except Exception as e:sql_errors.append({'file':str(f.relative_to(p)),'sql':q,'error':str(e)})
# Imports outside strings are useful for auditing omitted parser dependencies.
imports={}
for f,c in codes.items():
    roots=sorted(set(re.findall(r'\b(?:use|extern crate)\s+([a-zA-Z_][a-zA-Z_0-9]*)',c)))
    imports[str(f.relative_to(p))]=roots
out={'rust_files':len(rustfiles),'delimiter_findings':issues,'static_sql_compiled':count,'sql_compile_findings':sql_errors,'external_import_roots':sorted(set(x for v in imports.values() for x in v))}

print(json.dumps(out,indent=2))

raise SystemExit(1 if issues or sql_errors else 0)
