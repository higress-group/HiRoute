#!/usr/bin/env python3
"""Deterministic article figures from verified published results. Requires matplotlib."""
import json
from pathlib import Path
from html import escape
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
from reproduce import ROOT, RESULTS, report


def render_handoffs(out, result):
    evidence = json.loads((ROOT / 'cases/unattended-engineering/results/2026-10-04/evidence.json').read_text())
    stages = []
    for request in evidence['requests']:
        model, = request['models']
        if not stages or stages[-1] != model:
            stages.append(model)
    if stages != ['qwen3.8-flash', 'gpt-6-astra'] * 2:
        raise ValueError('The recorded model sequence changed; review the handoff narrative.')
    changes = len(stages) - 1
    checks = result['unattended']['independent_assertions']['total_passed']
    first_upgrade, final_upgrade = [decision for decision in evidence['decisions'] if decision['reason'] == 'competence_guard']
    for lang in ['zh', 'en']:
        zh = lang == 'zh'
        title = f'一项长任务，{changes} 次自动模型接力' if zh else f'One long task. {changes} automatic model handoffs.'
        names = ['Qwen 起步', 'Astra 整理交接', 'Qwen 接续', 'Astra 完成交付'] if zh else ['Qwen starts', 'Astra summarizes', 'Qwen continues', 'Astra delivers']
        roles = ['阅读源码与测试', '完成上下文摘要', '继续研究与探查', '实现、测试与修复'] if zh else ['Read source and tests', 'Prepare context summary', 'Continue investigation', 'Implement, test and repair']
        details = ['经济模型优先起步', '', '摘要交接后按策略回切', ''] if zh else ['Economy-first policy', '', 'Summary complete · return', '']
        for index, decision in [(1, first_upgrade), (3, final_upgrade)]:
            details[index] = f"{decision['competence']} < {decision['competence_floor']}" + (' · 升级' if zh else ' · upgrade')
        summary = (f'{changes} 次自动切换 · 零中途人工提示 · {checks} 项独立验收通过' if zh else f'{changes} automatic model changes · No operator prompts during execution · {checks} passing checks')
        note = '31 分 36 秒：阶段反馈触发升级，交接后按策略回切，进展不足时再次升级。' if zh else '31m 36s: upgrade on stage feedback, return after handoff, upgrade again when progress remains insufficient.'
        svg = ['<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1240 350" role="img">',
               f'<title>{escape(title)}</title><rect width="1240" height="350" rx="20" fill="#eef2fa"/>',
               '<g font-family="system-ui,sans-serif" fill="#192439">',
               f'<text x="32" y="48" font-size="26" font-weight="700">{escape(title)}</text>']
        for i, x in enumerate([32, 340, 648, 956]):
            svg.append(f'<rect x="{x}" y="85" width="252" height="142" rx="14" fill="white"/>')
            for y, size, weight, value in [(122, 20, 700, names[i]), (162, 17, 400, roles[i]), (198, 15, 400, details[i])]:
                svg.append(f'<text x="{x+16}" y="{y}" font-size="{size}" font-weight="{weight}">{escape(value)}</text>')
            if i < len(stages) - 1:
                svg.append(f'<path d="M {x+261} 151 h 38 m -10 -7 l 10 7 -10 7" fill="none" stroke="#586caa" stroke-width="3"/>')
        svg.extend([f'<text x="32" y="277" font-size="22" font-weight="700">{escape(summary)}</text>',
                    f'<text x="32" y="316" font-size="16" fill="#536079">{escape(note)}</text>', '</g></svg>'])
        (out / f'unattended-handoff-{lang}.svg').write_text(''.join(svg) + '\n')


def render():
    result = report()
    out = ROOT.parent / 'news/assets'
    out.mkdir(parents=True, exist_ok=True)
    rows = json.loads((RESULTS/'deliveries.json').read_text())['rows']
    plt.rcParams.update({'svg.hashsalt':'hiroute-news-20261004','svg.fonttype':'none','font.family':'DejaVu Sans'})
    from decimal import Decimal
    accepted = [p['pair'] for p in result['pairs'] if p['accepted']]
    costs = {}
    for group in ['strong', 'mixed']:
        selected = [next(r for r in rows if r['id'] == f"formal-draft98-v1-p{number:02d}-{group}") for number in accepted]
        costs[group] = tuple(sum(Decimal(row['usd'][bound]) for row in selected) / len(selected) for bound in ['lower', 'upper'])
    ratio = costs['mixed'][1] / costs['strong'][0]
    for lang in ['zh', 'en']:
        # SVG text remains selectable. Browser font fallback renders the Chinese
        # labels without requiring a platform-specific font in the generator.
        fig, ax = plt.subplots(figsize=(10, 4.8))
        zh = lang == 'zh'
        labels = ['全 Astra', 'HiRoute 混合'] if zh else ['All Astra', 'HiRoute mixed']
        for n, group in enumerate(['strong', 'mixed']):
            lo, hi = costs[group]
            percent = 100 if group == 'strong' else float(ratio * 100)
            color = '#5564bd' if group == 'strong' else '#078677'
            ax.barh(n, percent, height=.46, color=color)
            ax.text(percent + 2, n, f"${lo:.3f}–{hi:.3f}", va='center', fontsize=12, fontweight='bold', color='#192439')
        ax.set_yticks([0, 1], labels); ax.invert_yaxis(); ax.set_xlim(0, 132)
        ax.set_xticks([0, 25, 50, 75, 100], ['0%', '25%', '50%', '75%', '100%'])
        ax.set_xlabel('整单 API 等价成本 · 全 Astra = 100%' if zh else 'Whole-task API-equivalent cost · All Astra = 100%')
        ax.spines[['top', 'right', 'left']].set_visible(False)
        ax.set_title('相同交付标准，成本降低超过 90%' if zh else 'Same delivery standard. Over 90% lower cost.', loc='left', pad=20, fontsize=19)
        ax.text(130, 1, f"−{(1-ratio)*100:.2f}%", va='center', ha='right', fontsize=23, fontweight='bold', color='#087464')
        note = ('两次双方均达标运行的平均成本，包含全部尝试与路由评估。\n保守比较：混合成本上界 ÷ Astra 成本下界；完整三次结果见实验记录。' if zh else 'Mean cost across two runs meeting the same gate, including all attempts and routing evaluation.\nConservative comparison: mixed upper bound ÷ Astra lower bound. All three runs remain in the record.')
        fig.text(.08, .035, note, fontsize=9, color='#566078')
        fig.tight_layout(rect=[0, .14, 1, 1])
        target = out / f'research-cost-{lang}.svg'
        fig.savefig(target, metadata={'Date': None})
        target.write_text('\n'.join(line.rstrip() for line in target.read_text().splitlines()) + '\n')
        plt.close(fig)
    render_handoffs(out, result)


if __name__=='__main__':render()
