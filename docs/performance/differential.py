"""Compare baseline/candidate CLI contracts at identical source paths.

在相同源码路径逐项比较基线与候选 CLI 的输出、来源信息和诊断。
Usage / 用法: python docs/performance/differential.py BASELINE CANDIDATE
Only executable names and line endings in console streams are normalized;
artifact bytes, diagnostic frames, budgets and report statistics stay exact.
仅规范化控制台换行及可执行文件名称；产物字节、诊断帧、预算和统计严格比较。
"""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile


NS = 'xmlns:xs="https://xmlsquish.moesegfault.dev/ns" xmlns:m="urn:diff"'


def wrap(kind, body):
    """Wrap source without changing body positions. / 包装源码，保留正文位置。"""
    return f"<xs:{kind} {NS}>{body}</xs:{kind}>"


def fixtures():
    """Yield semantic and resource-boundary cases. / 枚举语义及资源边界样例。"""
    echo = '<xs:macro name="m:echo"><xs:param name="x"/><xs:insert get="arg.x"/></xs:macro>'
    rec = ('<xs:macro name="m:rec"><xs:param name="x"/>'
           '<xs:ifr get="arg.x" pattern="^(?&lt;head&gt;.)(?&lt;tail&gt;.*)$">'
           '<xs:insert get="match.head"/><xs:expand ref="m:rec">'
           '<xs:arg name="x" get="match.tail"/></xs:expand></xs:ifr></xs:macro>')
    expand = '<xs:expand ref="m:echo"><xs:arg name="x" value="雪 &amp; &lt;🐈&gt;"/></xs:expand>'
    fill = ('<xs:macro name="m:fill"><A><xs:slot name="body" required="true"/></A>'
            '</xs:macro>')
    cases = [
        ('repeat-args', echo, '<R>' + expand * 16 + '</R>', [], True),
        ('fill-origin', fill, '<R><xs:expand ref="m:fill"><xs:fill name="body">'
         '<V a="metadata">雪 &amp; 🐈</V></xs:fill></xs:expand>'
         '<xs:expand ref="m:fill"><xs:fill name="body">'
         '<V a="metadata">雪 &amp; 🐈</V></xs:fill></xs:expand></R>', [], True),
        ('recursive-return', echo + rec, '<R><xs:expand ref="m:echo"><xs:arg name="x">'
         '<xs:expand ref="m:rec"><xs:arg name="x" value="abc雪🐈"/></xs:expand>'
         '</xs:arg></xs:expand></R>', [], True),
        ('xml-escaping', '', '<m:R xml:space="preserve" a="&quot;&amp;">  雪 &lt; &amp; '
         '<![CDATA[<猫>]]> <X/> </m:R>', [], True),
        ('invalid-arg', echo, '<R><xs:expand ref="m:echo">'
         '<xs:arg name="wrong" value="x"/></xs:expand></R>', [], False),
        ('missing-root-arg', '', '<xs:param name="x"/><R><xs:insert get="arg.x"/></R>', [], False),
        ('isolated-scope', echo, '<xs:param name="x"/><R><xs:expand ref="m:echo"/></R>',
         ['--arg', 'x=outer'], False),
        ('required-fill', fill, '<R><xs:expand ref="m:fill"/></R>', [], False),
    ]
    for name, library, body, args, success in cases:
        yield name, {'lib.xml': wrap('module', library), 'input.xml': wrap('entry',
                    '<xs:import src="lib.xml"/>' + body)}, args, success
    for flag in ['--max-depth', '--max-expansions']:
        for limit in [0, 1, 2, 3, 4, 5]:
            yield f'{flag[2:]}-{limit}', {
                'lib.xml': wrap('module', rec),
                'input.xml': wrap('entry', '<xs:import src="lib.xml"/><R>'
                    '<xs:expand ref="m:rec"><xs:arg name="x" value="ab"/></xs:expand></R>')
            }, [flag, str(limit)], limit >= 4
    # Discover the baseline's exact serialized-byte threshold rather than guessing
    # namespaces or serializer spacing. / 在 main 中探测精确字节阈值，不猜测序列化开销。
    yield 'byte-probe', {'input.xml': wrap('entry', '<R>雪&amp;</R>')}, [], True
    yield 'diamond-import', {
        'input.xml': wrap('entry', '<xs:import src="left.xml"/><xs:import src="right.xml"/>'
                          '<R>' + expand + '</R>'),
        'left.xml': wrap('module', '<xs:import src="lib.xml"/>'),
        'right.xml': wrap('module', '<xs:import src="lib.xml"/>'),
        'lib.xml': wrap('module', echo),
    }, [], True


def run(exe, directory, args):
    """Clear only known outputs, then capture a run. / 仅清除已知产物并捕获执行结果。"""
    products = [directory / f'input.{suffix}.xml' for suffix in ('o', 'i')]
    for product in products:
        product.unlink(missing_ok=True)
    result = subprocess.run([str(exe), '--color=never', '--debug', *args,
                             str(directory / 'input.xml')], cwd=directory,
                            capture_output=True, timeout=60, check=False)
    def normalize(data):
        """Normalize process spelling, not diagnostics. / 仅规范进程名称，不删除诊断。"""
        return data.replace(b'\r\n', b'\n').replace(str(exe).encode(), b'<CLI>').replace(exe.name.encode(), b'<CLI>')
    return {'exit': result.returncode, 'stdout': normalize(result.stdout),
            'stderr': normalize(result.stderr),
            **{p.name: p.read_bytes() if p.exists() else None for p in products}}


def main():
    """Run both binaries sequentially at one absolute path. / 在同一绝对路径依次运行两份二进制。"""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('baseline', type=Path)
    parser.add_argument('candidate', type=Path)
    parser.add_argument('--report', type=Path)
    options = parser.parse_args()
    baseline, candidate = options.baseline.resolve(strict=True), options.candidate.resolve(strict=True)
    results = []
    with tempfile.TemporaryDirectory(prefix='xmlsquish-diff-') as temp:
        root = Path(temp)
        cases = list(fixtures())
        for name, files, args, success in cases:
            directory = root / name
            directory.mkdir()
            for path, source in files.items():
                (directory / path).write_text(source, encoding='utf-8')
            before = run(baseline, directory, args)
            if name == 'byte-probe':
                low, high = 0, 1024
                while low < high:
                    middle = (low + high) // 2
                    probe = run(baseline, directory, ['--max-output-bytes', str(middle)])
                    if probe['exit'] == 0:
                        high = middle
                    else:
                        low = middle + 1
                for limit in [low - 1, low, low + 1]:
                    cases.append((f'bytes-{limit}', files, ['--max-output-bytes', str(limit)], limit >= low))
            after = run(candidate, directory, args)
            differences = [key for key in before if before[key] != after[key]]
            if (before['exit'] == 0) != success:
                raise AssertionError(f'Baseline fixture {name} violates expected success={success}: {before}')
            results.append({'case': name, 'equal': not differences, 'differences': differences,
                            'baseline_exit': before['exit'],
                            'artifacts': {key: hashlib.sha256(value).hexdigest()
                                for key, value in before.items() if key.endswith('.xml') and value is not None}})
            print(f'{"PASS" if not differences else "FAIL"} {name}: {differences}')
            if differences:
                print(repr(before), repr(after))
    report = {'baseline': str(baseline), 'candidate': str(candidate), 'cases': results}
    if options.report:
        options.report.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    raise SystemExit(0 if all(item['equal'] for item in results) else 1)


if __name__ == '__main__':
    main()
