# Checkout total race: regression test

The new test `checkout renders totals after the price lookup` ran 20 times
on the executor without a failure; the old assertion failed 4 of 20.

| run | before | after |
| --- | --- | --- |
| 20 | 4 failed | 0 failed |

Screenshot: `checkout-total.png` shows the total once the lookup resolved.
