const test = require('node:test');
const assert = require('node:assert/strict');

const { calculateStakingYield } = require('../lib/stakingCalculator');

test('StakingCalculator logic integration: default parameters yield calculation', () => {
  const result = calculateStakingYield();
  assert.equal(result.depositAmount, 1000);
  assert.equal(result.lockDurationMonths, 12);
  assert.equal(result.baseApyPercentage, 12);
  assert.equal(result.effectiveApyPercent, 18); // 12% * 1.5 multiplier
  assert.ok(result.totalBalance > 1000);
  assert.ok(result.totalInterest > 0);
});

test('StakingCalculator logic integration: max lock duration (36 months)', () => {
  const result = calculateStakingYield({
    depositAmount: 5000,
    lockDurationMonths: 36,
    baseApyPercentage: 10,
    compoundFrequency: 'daily',
    enableTierMultiplier: true,
  });

  // For 36 months, tier multiplier is 2.0x -> effective APY is 20%
  assert.equal(result.multiplier, 2.0);
  assert.equal(result.effectiveApyPercent, 20);
  assert.ok(result.totalBalance > 5000 * 1.6);
  assert.equal(result.breakdownByMonth.length, 36);
});


test('StakingCalculator logic integration: compound frequency changes the projected return', () => {
  const base = {
    depositAmount: 10000,
    lockDurationMonths: 12,
    baseApyPercentage: 10,
    enableTierMultiplier: false,
  };

  const simple = calculateStakingYield({ ...base, compoundFrequency: 'none' });
  const monthly = calculateStakingYield({ ...base, compoundFrequency: 'monthly' });
  const daily = calculateStakingYield({ ...base, compoundFrequency: 'daily' });

  assert.equal(simple.totalBalance, 11000);
  assert.ok(monthly.totalBalance > simple.totalBalance);
  assert.ok(daily.totalBalance >= monthly.totalBalance);
});

test('StakingCalculator logic integration: milestone table contains monthly projections', () => {
  const result = calculateStakingYield({
    depositAmount: 2500,
    lockDurationMonths: 6,
    baseApyPercentage: 8,
    compoundFrequency: 'weekly',
  });

  assert.equal(result.breakdownByMonth.length, 6);
  assert.equal(result.breakdownByMonth[0].month, 1);
  assert.equal(result.breakdownByMonth.at(-1).month, 6);
  assert.ok(result.breakdownByMonth.at(-1).balance > result.breakdownByMonth[0].balance);
});
