import { it } from 'vitest';
import { writeFileSync } from 'node:fs';
import { Shogi } from '@shogitter/core';
import { completedOrdinary } from './engines/ordinaryRuleFixtures.js';
import { ORDINARY_RULES } from './engines/ordinaryRules.js';
const rows: unknown[] = [];
for (const rule of ORDINARY_RULES) {
  it(`initial completed-turn perft(1..2): ${rule.name}`, () => {
    const game = Shogi.ofRuleId(rule.ruleId);
    game.start();
    const moves = completedOrdinary(game);
    const second = moves.reduce((n, t) => n + completedOrdinary(t.shogi).length, 0);
    rows.push({name: rule.name, ruleId: rule.ruleId, perft: {'1':moves.length, '2':second}});
    writeFileSync('core-perft-reference.json', JSON.stringify(rows,null,2)+'\n');
    console.log(rule.name, moves.length, second);
  }, 600000);
}
