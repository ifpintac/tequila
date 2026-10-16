/* Prints reference values from the official Hosek-Wilkie 1.4a code, for
   Tequila's Rust port to be tested against (tests/hosek_reference.rs).
   Build beside the authors' unmodified source, from
   https://cgg.mff.cuni.cz/projects/SkylightModelling/, e.g. with MSVC:
   cl /O2 /fp:precise hosek_harness.c ArHosekSkyModel.c, and run with its
   output redirected to hosek_reference.txt. */
#include <stdio.h>
#include <math.h>
#include "ArHosekSkyModel.h"

double arhosekskymodel_solar_radiance_internal2(ArHosekSkyModelState *state, double wavelength, double elevation, double gamma);

int main(void)
{
    const double turbidities[] = {1.0, 2.0, 2.5, 5.3, 9.0, 10.0};
    const double albedos[] = {0.0, 0.2, 1.0};
    const double elevations[] = {0.0, 0.05, 0.3, 0.8, 1.5};
    const double views[][2] = {{0.0, 0.3}, {0.7, 0.05}, {1.2, 1.0}, {1.55, 2.5}, {1.5707, 0.9}, {0.4, 3.0}};
    /* rgb: T A E theta gamma r g b */
    for (int t = 0; t < 6; t++)
        for (int a = 0; a < 3; a++)
            for (int e = 0; e < 5; e++) {
                ArHosekSkyModelState *s = arhosek_rgb_skymodelstate_alloc_init(turbidities[t], albedos[a], elevations[e]);
                for (int v = 0; v < 6; v++)
                    printf("rgb %.3f %.3f %.3f %.4f %.4f %.17g %.17g %.17g\n", turbidities[t], albedos[a], elevations[e],
                           views[v][0], views[v][1],
                           arhosek_tristim_skymodel_radiance(s, views[v][0], views[v][1], 0),
                           arhosek_tristim_skymodel_radiance(s, views[v][0], views[v][1], 1),
                           arhosek_tristim_skymodel_radiance(s, views[v][0], views[v][1], 2));
                arhosekskymodelstate_free(s);
            }
    /* spectral sky and sun: T A E theta gamma wl sky sun_direct_at_centre */
    for (int t = 0; t < 6; t++)
        for (int e = 1; e < 5; e++) {
            ArHosekSkyModelState *s = arhosekskymodelstate_alloc_init(elevations[e], turbidities[t], 0.2);
            for (int w = 0; w < 11; w++) {
                double wl = 320.0 + 40.0 * w;
                printf("spec %.3f %.3f %.3f %.4f %.4f %.1f %.17g %.17g\n", turbidities[t], 0.2, elevations[e], 0.7, 0.05, wl,
                       arhosekskymodel_radiance(s, 0.7, 0.05, wl),
                       arhosekskymodel_solar_radiance_internal2(s, wl, elevations[e], 0.0));
            }
            arhosekskymodelstate_free(s);
        }
    return 0;
}
