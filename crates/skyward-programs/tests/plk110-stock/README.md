# OnePlus 15 service discovery fixture

These test inputs model the services observed on PLK110 with vendor SKU
`canoe` and hardware SKU `23`. They cover the Qualcomm key chain, NXP
Weaver and OMAPI dependencies, device accounts, and the three Android,
Qualcomm and Oplus supplicant declarations. Empty files under `bin`
represent the existence of service programs.

The init fixture also includes a line beginning with `//` before the
first section to exercise the parser's handling of stray vendor lines.
